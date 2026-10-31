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
//!
//! **The lens lives on the conversation, not the run.** In a chat one
//! message is one run, so a per-run lens judged every turn's first request
//! as a [`Verdict::Baseline`] — and the first request after an idle gap is
//! exactly where a server that evicted the chat's prefix would show. Carried
//! across runs, that request is judged against the previous run's last one,
//! and [`CacheLens::resume`] keeps the verdict so the run's outcome row can
//! record it ([`ResumeCache`]): the measurement `docs/IDLE-CACHE-DESIGN.md`
//! waits on before anything saves a chat's state to disk.

use crate::message::{CompletionRequest, Usage};
use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

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
    /// The tool list, system prompt or model changed since the last request.
    /// Reuse breaking here is expected; the name says which knob moved. A
    /// model switch reloads the server, and nothing it held survives.
    SurfaceChanged {
        system: bool,
        tools: bool,
        model: bool,
    },
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

#[derive(Debug, Clone)]
struct Prev {
    system: u64,
    tools: u64,
    model: u64,
    /// When this request's usage came back: the start of the idle gap the
    /// next run's first request is measured across.
    at: Instant,
    /// One hash per message *as sent*, less the run's notes
    /// (`CompletionRequest::trailing_notes`), so the append-only check is a
    /// prefix comparison rather than a diff.
    messages: Vec<u64>,
    total_input: u64,
    /// The part of `total_input` that was the request's notes, estimated by
    /// their share of the request's bytes. Notes ride the end of the last
    /// message and move with each request, so the next one re-reads them by
    /// design, and that is not reuse failing.
    notes_input: u64,
}

#[derive(Debug, Clone, Default)]
pub struct CacheLens {
    prev: Option<Prev>,
    /// A nonzero cache tier has been reported at least once, so zeros from
    /// here on are evidence rather than silence.
    reporting_seen: bool,
    /// Set by [`CacheLens::begin_run`]; the next observation is the run's
    /// first request, and its verdict is kept in `resume`.
    awaiting_first: bool,
    resume: Option<Resume>,
}

/// How a run's first request met the cache, with the gap since the request
/// before it.
#[derive(Debug, Clone, PartialEq)]
pub struct Resume {
    pub verdict: Verdict,
    /// `None` when there was no earlier request to measure from.
    pub idle: Option<Duration>,
    pub read: u64,
    pub uncached: u64,
}

impl CacheLens {
    pub fn new() -> Self {
        Self::default()
    }

    /// A run is starting on this conversation: its first observation is the
    /// one [`CacheLens::resume`] reports. The comparison state is kept — that
    /// is the point of carrying the lens across runs.
    pub fn begin_run(&mut self) {
        self.awaiting_first = true;
        self.resume = None;
    }

    /// The verdict on the current run's first request, once it has been
    /// observed. `None` before then, and for a run that sent nothing.
    pub fn resume(&self) -> Option<&Resume> {
        self.resume.as_ref()
    }

    /// Observe one completed request and the usage the provider reported for
    /// it. Pure bookkeeping: the caller decides what, if anything, to do
    /// with the verdict.
    pub fn observe(&mut self, request: &CompletionRequest, usage: &Usage) -> Verdict {
        let now = Instant::now();
        self.observe_at(request, usage, now, now)
    }

    /// [`CacheLens::observe`] for a request sent at `sent`. The idle gap a
    /// run's first request records ends there, not when its answer came
    /// back: measured to the response, it would include the request's own
    /// prompt read — two minutes on a 176k re-read, and largest on exactly
    /// the misses the gap is recorded to explain.
    pub fn observe_sent(
        &mut self,
        request: &CompletionRequest,
        usage: &Usage,
        sent: Instant,
    ) -> Verdict {
        self.observe_at(request, usage, sent, Instant::now())
    }

    /// The same with both instants passed in, so a test can put an idle gap
    /// between two runs without sleeping through it: `sent` ends the gap
    /// before this request, `done` starts the one after it.
    pub fn observe_at(
        &mut self,
        request: &CompletionRequest,
        usage: &Usage,
        sent: Instant,
        done: Instant,
    ) -> Verdict {
        let (messages, notes_bytes) = history_hashes(request);
        let total_bytes = request.system.as_ref().map_or(0, String::len)
            + serde_json::to_string(&request.tools).map_or(0, |t| t.len())
            + crate::pressure::message_bytes(&request.messages);
        let total_input = usage.total_input();
        let current = Prev {
            system: hash_of(&request.system),
            // The tool choice rides with the tools: a provider that keys its
            // message cache on it (Anthropic lists it as invalidating the
            // message blocks) re-reads the history on a closing request, and
            // that is a surface change the harness made, never the
            // unexplained drop this lens exists to isolate (review of #596).
            tools: hash_of(&(&request.tools, format!("{:?}", request.tool_choice))),
            model: hash_of(&request.model),
            at: done,
            messages,
            total_input,
            notes_input: (total_input as f64 * notes_bytes as f64 / total_bytes.max(1) as f64)
                as u64,
        };
        let cached_reported =
            usage.cache_read_input_tokens > 0 || usage.cache_creation_input_tokens > 0;

        let verdict = match &self.prev {
            None => Verdict::Baseline,
            Some(prev) => {
                let system = prev.system != current.system;
                let tools = prev.tools != current.tools;
                let model = prev.model != current.model;
                if system || tools || model {
                    Verdict::SurfaceChanged {
                        system,
                        tools,
                        model,
                    }
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
                        .saturating_sub(usage.cache_read_input_tokens)
                        .saturating_sub(prev.notes_input);
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

        if std::mem::take(&mut self.awaiting_first) {
            self.resume = Some(Resume {
                verdict: verdict.clone(),
                idle: self
                    .prev
                    .as_ref()
                    .map(|p| sent.saturating_duration_since(p.at)),
                read: usage.cache_read_input_tokens,
                uncached: usage.input_tokens,
            });
        }
        self.reporting_seen |= cached_reported;
        self.prev = Some(current);
        verdict
    }
}

/// A run's [`Resume`] as its outcome row records it (`RunStats::resume_cache`).
///
/// Numbers and a closed verdict, never content: the lens holds hashes only,
/// and the row is a corpus input like every other `RunStats` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeCache {
    pub verdict: ResumeVerdict,
    /// Seconds since the conversation's previous request came back. Absent
    /// on a baseline: the first request this process saw for the
    /// conversation, so a new chat or one reopened after a restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_secs: Option<u64>,
    /// On `surface_changed`, which knobs moved.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub system_changed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tools_changed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub model_changed: bool,
    /// On `drop`: the part of the previous prompt that was paid for again,
    /// and that prompt's size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repaid: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_total: Option<u64>,
    /// What the first request read from the cache and what it did not.
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub uncached: u64,
    /// The agent shares the server's slots with interleaved conversations
    /// (`Agent::set_cache_contended`), so a drop has a known cause.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub contended: bool,
}

/// [`Verdict`]'s kind, as a closed set written to an append-only store: an
/// unknown name read back is [`ResumeVerdict::Unknown`], never a failed row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeVerdict {
    Baseline,
    Stable,
    SurfaceChanged,
    TranscriptRewritten,
    Unobservable,
    Drop,
    #[serde(other)]
    Unknown,
}

impl Resume {
    pub fn record(&self, contended: bool) -> ResumeCache {
        let mut r = ResumeCache {
            verdict: ResumeVerdict::Baseline,
            idle_secs: self.idle.map(|d| d.as_secs()),
            system_changed: false,
            tools_changed: false,
            model_changed: false,
            repaid: None,
            prev_total: None,
            cache_read: self.read,
            uncached: self.uncached,
            contended,
        };
        r.verdict = match &self.verdict {
            Verdict::Baseline => ResumeVerdict::Baseline,
            Verdict::Stable { .. } => ResumeVerdict::Stable,
            Verdict::SurfaceChanged {
                system,
                tools,
                model,
            } => {
                (r.system_changed, r.tools_changed, r.model_changed) = (*system, *tools, *model);
                ResumeVerdict::SurfaceChanged
            }
            Verdict::TranscriptRewritten => ResumeVerdict::TranscriptRewritten,
            Verdict::Unobservable => ResumeVerdict::Unobservable,
            Verdict::Drop { repaid, prev_total } => {
                (r.repaid, r.prev_total) = (Some(*repaid), Some(*prev_total));
                ResumeVerdict::Drop
            }
        };
        r
    }
}

/// Hash anything serializable. `DefaultHasher` is unstable across processes,
/// which is fine: a lens lives inside one process (it rides a `Conversation`
/// across runs, never a file) and its hashes never leave it.
fn hash_of<T: serde::Serialize>(value: &T) -> u64 {
    let mut h = DefaultHasher::new();
    serde_json::to_string(value)
        .unwrap_or_default()
        .hash(&mut h);
    h.finish()
}

/// One hash per message, the last with its trailing notes left out, and the
/// notes' bytes. Within a run the notes move from the owner's turn to the
/// tool results; hashed in place, every request after the first would read
/// as a rewritten transcript, and the lens could never call a drop on a run
/// that carries notes.
fn history_hashes(request: &CompletionRequest) -> (Vec<u64>, usize) {
    let mut hashes: Vec<u64> = request.messages.iter().map(hash_of).collect();
    let Some(last) = request.messages.last() else {
        return (hashes, 0);
    };
    let keep = last.content.len().saturating_sub(request.trailing_notes);
    if keep == last.content.len() {
        return (hashes, 0);
    }
    let notes_bytes = last.content[keep..]
        .iter()
        .map(crate::pressure::block_bytes)
        .sum();
    let mut stripped = last.clone();
    stripped.content.truncate(keep);
    let n = hashes.len();
    if stripped.content.is_empty() {
        hashes.pop();
    } else {
        hashes[n - 1] = hash_of(&stripped);
    }
    (hashes, notes_bytes)
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
            response_schema: None,
            model: "m".into(),
            system: Some("system".into()),
            messages,
            tools: vec![],
            max_tokens: 512,
            effort: None,
            thinking: false,
            cache_prompt: true,
            think: None,
            think_budget: None,
            local_only: false,
            trailing_notes: 0,
            tool_choice: crate::message::ToolChoice::Auto,
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

    /// A run's notes ride the last message and move to the tool results
    /// after a round trip (`message::attach_notes`). The transcript only
    /// grew, so the lens still judges reuse, and the notes' own re-read is not
    /// called a drop, while a real drop on such a run still is.
    #[test]
    fn a_runs_moving_notes_neither_hide_a_drop_nor_fake_one() {
        let note = crate::message::Block::text("n".repeat(3_000));
        let with_note = |mut messages: Vec<Message>| {
            messages.last_mut().unwrap().content.push(note.clone());
            CompletionRequest {
                trailing_notes: 1,
                tool_choice: crate::message::ToolChoice::Auto,
                ..request(messages)
            }
        };
        let first = with_note(vec![Message::user("hi")]);
        let second = with_note(vec![
            Message::user("hi"),
            Message::assistant(vec![crate::message::Block::text("calling")]),
            Message::user("tool output"),
        ]);
        let mut lens = CacheLens::new();
        assert_eq!(
            lens.observe(&first, &usage(10, 2_000, 0)),
            Verdict::Baseline
        );
        // Everything but the note came back: about 3,000 of the first
        // request's bytes were the note, so ~1,900 tokens re-read is it.
        assert!(matches!(
            lens.observe(&second, &usage(2_100, 0, 110)),
            Verdict::Stable { .. }
        ));
        // And a run with notes whose history did not come back is a drop.
        let mut lens = CacheLens::new();
        let big = |n| {
            let mut m = convo(n);
            m.push(Message::user("x".repeat(40_000)));
            m
        };
        lens.observe(&with_note(big(1)), &usage(10, 12_000, 0));
        let mut grown = big(1);
        grown.push(Message::assistant(vec![crate::message::Block::text("ok")]));
        grown.push(Message::user("more"));
        assert!(matches!(
            lens.observe(&with_note(grown), &usage(12_000, 0, 0)),
            Verdict::Drop { .. }
        ));
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
                tools: true,
                model: false
            }
        );
    }

    /// A closing request asks for words (`ToolChoice::None`): a provider
    /// that re-reads the history for it is reading a change the harness
    /// made, said as one (review of #596).
    #[test]
    fn a_changed_tool_choice_is_an_expected_break_not_a_drop() {
        let mut lens = CacheLens::new();
        lens.observe(&request(convo(1)), &usage(8, 18_000, 0));
        let mut closing = request(convo(2));
        closing.tool_choice = crate::message::ToolChoice::None;
        assert_eq!(
            lens.observe(&closing, &usage(18_000, 500, 0)),
            Verdict::SurfaceChanged {
                system: false,
                tools: true,
                model: false
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

    /// The case the lens moved onto the conversation for: a chat's next
    /// message is a new run, and its first request — after however long the
    /// chat sat idle — is judged against the previous run's last one. Built
    /// per run, that request was always a baseline and a server that lost the
    /// prefix in between was invisible.
    #[test]
    fn a_resumed_run_whose_prefix_the_server_lost_is_a_drop_with_its_idle_gap() {
        let t0 = Instant::now();
        let mut lens = CacheLens::new();
        lens.begin_run();
        lens.observe_at(&request(convo(1)), &usage(8, 18_000, 0), t0, t0);
        lens.observe_at(&request(convo(2)), &usage(40, 200, 18_000), t0, t0);
        // Next message, twenty minutes later: nothing changed, the transcript
        // only grew, and the cache handed back none of it.
        lens.begin_run();
        let later = t0 + Duration::from_secs(1_200);
        assert!(matches!(
            lens.observe_at(&request(convo(3)), &usage(18_300, 0, 0), later, later),
            Verdict::Drop { .. }
        ));
        let resume = lens.resume().unwrap().record(false);
        assert_eq!(resume.verdict, ResumeVerdict::Drop);
        assert_eq!(resume.idle_secs, Some(1_200));
        assert_eq!(resume.prev_total, Some(18_240));
        assert_eq!(resume.cache_read, 0);
    }

    /// The gap ends when the resuming request is sent. Measured to its
    /// response, a re-read's own minutes of prompt processing would count as
    /// idle time, inflating exactly the rows a drop is recorded on.
    #[test]
    fn the_idle_gap_ends_at_the_send_not_the_response() {
        let t0 = Instant::now();
        let mut lens = CacheLens::new();
        lens.begin_run();
        lens.observe_at(&request(convo(1)), &usage(8, 18_000, 0), t0, t0);
        lens.begin_run();
        let sent = t0 + Duration::from_secs(600);
        let done = sent + Duration::from_secs(130);
        lens.observe_at(&request(convo(2)), &usage(18_300, 0, 0), sent, done);
        assert_eq!(lens.resume().unwrap().record(false).idle_secs, Some(600));
    }

    #[test]
    fn a_resumed_run_that_reused_the_prefix_is_stable() {
        let t0 = Instant::now();
        let mut lens = CacheLens::new();
        lens.begin_run();
        lens.observe_at(&request(convo(1)), &usage(8, 18_000, 0), t0, t0);
        lens.begin_run();
        lens.observe_at(
            &request(convo(2)),
            &usage(60, 0, 18_000),
            t0 + Duration::from_secs(30),
            t0 + Duration::from_secs(30),
        );
        let resume = lens.resume().unwrap().record(true);
        assert_eq!(resume.verdict, ResumeVerdict::Stable);
        assert_eq!((resume.idle_secs, resume.cache_read), (Some(30), 18_000));
        assert!(resume.contended);
    }

    /// The first request a process sees for a conversation has nothing to be
    /// compared with — a new chat, or one reopened after a restart — and says
    /// so rather than claiming an idle gap of zero.
    #[test]
    fn a_conversations_first_run_is_a_baseline_with_no_idle_gap() {
        let mut lens = CacheLens::new();
        lens.begin_run();
        lens.observe(&request(convo(1)), &usage(8, 18_000, 0));
        let resume = lens.resume().unwrap().record(false);
        assert_eq!(resume.verdict, ResumeVerdict::Baseline);
        assert_eq!(resume.idle_secs, None);
    }

    /// Only the run's first request is the resume: later turns of the same
    /// run are the per-run invariant the lens always watched.
    #[test]
    fn only_the_first_request_of_a_run_is_its_resume() {
        let mut lens = CacheLens::new();
        lens.begin_run();
        lens.observe(&request(convo(1)), &usage(8, 18_000, 0));
        lens.observe(&request(convo(2)), &usage(18_300, 0, 0));
        assert_eq!(lens.resume().unwrap().verdict, Verdict::Baseline);
        // And a run that has not sent anything yet reports nothing.
        lens.begin_run();
        assert!(lens.resume().is_none());
    }

    /// A model switch reloads the server, so the miss after it is a surface
    /// change with a name, never a drop.
    #[test]
    fn a_model_switch_between_runs_is_a_surface_change() {
        let mut lens = CacheLens::new();
        lens.begin_run();
        lens.observe(&request(convo(1)), &usage(8, 18_000, 0));
        lens.begin_run();
        let mut switched = request(convo(2));
        switched.model = "other".into();
        lens.observe(&switched, &usage(18_300, 0, 0));
        let resume = lens.resume().unwrap().record(false);
        assert_eq!(resume.verdict, ResumeVerdict::SurfaceChanged);
        assert!(resume.model_changed && !resume.tools_changed && !resume.system_changed);
    }

    /// A closed enum in an append-only store is a wire format: the record
    /// round-trips, and a verdict name this build does not know reads back as
    /// `Unknown` instead of failing the row it sits on.
    #[test]
    fn the_resume_record_round_trips_and_an_unknown_verdict_degrades() {
        let r = ResumeCache {
            verdict: ResumeVerdict::Drop,
            idle_secs: Some(600),
            system_changed: false,
            tools_changed: false,
            model_changed: false,
            repaid: Some(40_000),
            prev_total: Some(41_000),
            cache_read: 0,
            uncached: 41_200,
            contended: false,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("changed") && !json.contains("contended"));
        assert_eq!(serde_json::from_str::<ResumeCache>(&json).unwrap(), r);
        let future: ResumeCache =
            serde_json::from_str(r#"{"verdict":"evicted_to_disk","cache_read":5}"#).unwrap();
        assert_eq!(future.verdict, ResumeVerdict::Unknown);
        assert_eq!(future.cache_read, 5);
    }
}
