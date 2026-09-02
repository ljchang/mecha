//! Session transcripts.
//!
//! One JSONL file per run: a header line describing the session, then one line
//! per message. Append-only, so a crashed run still leaves a readable
//! transcript, and `mecha sessions resume` can pick it back up.

use crate::agent::{Agent, Conversation, Taint};
use crate::config::{Config, PermissionMode, TrifectaPolicy};
use crate::message::{Effort, Message, Usage};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A `message` record whose blocks a newer build wrote, read as far as this
/// build can.
///
/// `Block` is a closed enum written to an append-only store — a wire format
/// — and `serde` fails the whole record on a variant it does not know. Read
/// strictly, a message carrying one new block kind was *dropped from the
/// resumed conversation*, which can orphan a `tool_result` and 400 every
/// later request on the session. So a line that failed the strict parse is
/// retried block by block: the blocks this build knows are kept, the rest
/// are counted and logged, and the message survives with what it has. A
/// message left with no blocks at all, or a role this build does not know,
/// is still dropped — there is nothing left to keep.
///
/// The residue, stated: if a future build adds a block kind that *produces*
/// a result — a second `tool_use` shape — dropping it leaves the answering
/// `tool_result` orphaned in the next message, and nothing prunes orphans
/// at load. Strictly better than dropping the whole message either way,
/// and the fix when that day comes is a load-time orphan sweep beside this.
fn lenient_record(line: &str) -> Option<Record> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v.get("record").and_then(serde_json::Value::as_str)? {
        "message" => lenient_message(&v).map(Record::Message),
        // A rewrite carries a whole message list, and is the record where
        // dropping the line costs a compaction: the resumed conversation would
        // be the pre-compaction one, oversized, with the summary lost.
        "rewrite" => {
            let kept: Vec<Message> = v
                .get("messages")?
                .as_array()?
                .iter()
                .filter_map(lenient_message)
                .collect();
            (!kept.is_empty()).then_some(Record::Rewrite { messages: kept })
        }
        _ => None,
    }
}

/// One message object — `{"role": …, "content": [...]}` — read as far as
/// this build can. See [`lenient_record`].
fn lenient_message(v: &serde_json::Value) -> Option<Message> {
    let role: crate::message::Role = serde_json::from_value(v.get("role")?.clone()).ok()?;
    let raw = v.get("content")?.as_array()?;
    let mut content = Vec::with_capacity(raw.len());
    let mut dropped = 0usize;
    for block in raw {
        match serde_json::from_value::<crate::message::Block>(block.clone()) {
            Ok(b) => content.push(b),
            Err(_) => dropped += 1,
        }
    }
    if content.is_empty() {
        return None;
    }
    if dropped > 0 {
        tracing::warn!(
            dropped,
            kept = content.len(),
            "a transcript message carried blocks this build cannot read; kept the rest"
        );
    }
    Some(Message { role, content })
}

/// Read a `stop_cause` that may have been written by a newer build.
///
/// Same rule as [`lenient_message`]: `StopCause` is a wire format, and a
/// variant this build does not know degrades to `None` rather than failing
/// the `Outcome` record — which would erase the run from the corpus on load.
fn lenient_stop_cause<'de, D>(
    d: D,
) -> std::result::Result<Option<crate::agent::StopCause>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum Record {
    Meta(SessionMeta),
    Message(Message),
    /// Written when a run finishes, so `sessions show` can report cost without
    /// replaying the whole transcript.
    Summary {
        usage: Usage,
        turns: u32,
    },
    /// How the run went, as distinct from what it said.
    ///
    /// [`Summary`] answers "what did this cost"; this answers "did it work".
    /// The distinction earns a second record because the audience is
    /// different: cost is for a person reading `sessions show`, and this is
    /// for a machine reading many sessions at once — the sensor a
    /// harness-improvement loop needs and did not have.
    ///
    /// The gap it closes is that [`crate::agent::RunOutcome`] carries fifteen
    /// fields and the transcript kept two of them, so every chat, TUI and
    /// Slack run was *less* observable than a trigger, whose ledger recorded
    /// the rest. The signal was already computed; it was thrown away at the
    /// end of every interactive run.
    ///
    /// [`Summary`]: Record::Summary
    Outcome(RunStats),
    /// Everything that shaped the request, written each time a process
    /// attaches to the session — on creation and again on every resume.
    ///
    /// Not folded into the header, because a session resumed under different
    /// flags would make a header written at creation a lie about every turn
    /// after the first. Within one process the configuration cannot change, so
    /// one record per attach is exactly the granularity that can differ.
    Config(RunConfig),
    /// What had entered the conversation by this point.
    ///
    /// Recorded because it cannot be recovered by reading the transcript back:
    /// taint keys off *provenance* — whether a result actually came from
    /// outside — and the transcript stores only the content. Without this,
    /// resuming a session that had read a hostile page would hand the model
    /// that page again with the interlock disarmed.
    Taint(Taint),
    /// The conversation's messages were rewritten in place — compaction
    /// summarised the head, eviction replaced a stale result, thinning
    /// shortened an old one. An append-only file cannot express an in-place
    /// rewrite as more `Message` records: slicing "what the run added" off
    /// the end of a rewritten list skips the rebuilt head, which is exactly
    /// where the compaction summary lives, and every trace of the rewrite
    /// with it — a 2026-08-07 benchmark transcript recorded 8 assistant turns
    /// of a 28-turn run that way, starting mid-conversation with no sign a
    /// compaction had ever happened. So the record carries the whole current
    /// list, and [`Session::load`] replaces what it has accumulated so far.
    Rewrite {
        messages: Vec<Message>,
    },
    /// A better name for this conversation than the one it was created with.
    ///
    /// **Appended, never patched.** The header is the first line of the file
    /// and [`Session::peek_meta`] reads exactly that line, which is what
    /// makes listing thousands of sessions cheap; rewriting it in place would
    /// trade an append-only store for one line of tidiness. So a rename is a
    /// record like everything else, and [`Session::read`] applies the last
    /// one it sees over the header's.
    ///
    /// A consequence worth knowing at the call sites: `peek_meta` and
    /// therefore [`Session::list`] still report the *created* title. That is
    /// deliberate rather than merely cheap — the created title is where the
    /// `web: ` / `voice: ` / `task: ` prefix that classifies a session comes
    /// from.
    ///
    /// **And that prefix *is* load-bearing, so a rename may not change it.**
    /// An earlier version of this comment said the opposite, which was the
    /// dangerous half-truth: `Session::read` applies renames, `load` is a
    /// thin wrapper over `read`, and `serve::chat`'s resume path feeds the
    /// loaded title straight to `task_withholding` — which gates
    /// `kg_task_update` on `task: ` and is how D6 (*the agent may not close
    /// its own task*) is enforced by absence. A rename that dropped the
    /// prefix would hand a resumed delegation back the tool that closes its
    /// own task. Today only `web: ` sessions are ever renamed and the rename
    /// re-stamps the prefix, so the hazard is one careless caller away
    /// rather than present — which is exactly the kind of thing that should
    /// not be guarded by a sentence in a doc comment. [`Session::read`]
    /// enforces it instead: a rename whose prefix disagrees with the
    /// header's is ignored.
    ///
    /// A struct variant, not a newtype one, because this enum is internally
    /// tagged: `#[serde(tag = "record")]` merges the variant's fields into
    /// the object beside the tag, and a bare `String` has no fields to merge
    /// — serialization fails at *runtime*, which for an append-only store is
    /// a record silently not written. Every other variant here happens to
    /// wrap a struct, so this is the first one that could have found out the
    /// hard way; `a_rename_survives_a_round_trip` is what does instead.
    Title {
        title: String,
    },
}

/// What a run was configured with, recorded so it can be replayed.
///
/// The rule behind the field list: **anything that shapes the request or
/// constrains the run is a confound if it is not recorded.** That is not
/// theoretical here — compaction on versus off measured 1/5 against 5/5 on the
/// same task, so a replay that did not know whether compaction was enabled
/// would compare two incomparable runs and report a model regression.
///
/// The system prompt is stored in full rather than hashed. A hash tells you
/// only *that* something differed; the text lets a replay rebuild the request.
/// It is no more sensitive than the transcript sitting beside it.
///
/// The sampler is recorded only as far as it is pinned: `temperature` and
/// `seed` hold what this process *sent*, and `None` means the server chose.
/// Replay against an unpinned run has to be pass@k-shaped rather than
/// exact-match-shaped; against a pinned, seeded run driven sequentially it can
/// expect to match. (Not greedy — temperature 0.0 walks qwen3.6 into verbatim
/// repetition loops. And only sequentially: llama-server's continuous batching
/// makes concurrent requests perturb each other's numerics, seed or no seed.)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RunConfig {
    /// Which harness produced this. The axis every replay diff is measured on.
    pub mecha_version: String,
    pub provider: String,
    pub model: String,
    pub workspace: PathBuf,
    /// The resolved text, not the path it may have come from.
    pub system_prompt: Option<String>,
    /// Tool names in registry order — which is the order they are sent, and the
    /// front of the cached prefix. A tool added, removed or renamed between
    /// recording and replay changes what the model could have done.
    pub tools: Vec<String>,
    /// The surface those names actually described, by hash.
    ///
    /// **Names were never enough, and the comment above says why without
    /// seeing it.** Add, remove and rename are the three that almost never
    /// happen; *re-describe* happens constantly — 49 commits touched tool
    /// definitions in three weeks of this store — and a list of names cannot
    /// see it. Tools render *before* the system prompt, so a replay was
    /// rebuilding the second half of the prefix byte-exactly and the first half
    /// from whatever the registry says today. Measured consequence: 12 of 13
    /// counterfactual probes inconclusive, deterministically, median divergence
    /// one tool call in.
    ///
    /// The specs themselves live in [`crate::surface::SurfaceStore`] — 69 KB
    /// against a 25 KB average session is why this is a citation and not the
    /// text, where `system_prompt` above is the text.
    ///
    /// **`None` is a recording from before this existed, and must never read as
    /// a match** — [`crate::surface::Fidelity`] is the three-state answer, and
    /// its `Unknown` arm is the one every session on disk today lands in.
    ///
    /// **Scope: `registry().specs()`, unfiltered — not necessarily what this
    /// turn's request actually sent.** The wire request goes through
    /// `registry.specs_for(cx.phase)`, which also applies `Phase::Plan`'s
    /// read-only filter and a loaded skill's `tools:` narrowing (matching this
    /// struct's own `tools` field, so this is not a new gap, only a named
    /// one). A run under `Plan`, or one that had a narrowing skill loaded,
    /// sent fewer specs than this hash covers — and since the surface can
    /// narrow *mid-run*, no single hash can describe every turn's request
    /// exactly. `Fidelity::Matches` here means "the full registry is
    /// unchanged since this was recorded", which is what makes a replay
    /// worth attempting; it is not a claim that the request bytes were
    /// identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_hash: Option<String>,

    // What the request looks like.
    pub effort: Option<Effort>,
    /// The temperature and seed actually sent, when the provider config pins
    /// them. Unset means the server chose, and the run is not repeatable.
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
    pub thinking: bool,
    /// No effect on semantics; large effect on the token counts a replay diffs.
    pub cache_prompt: bool,
    pub max_tokens: u32,

    // Ceilings. A run that hit one looks exactly like a model that gave up.
    pub max_turns: u32,
    pub max_output_tokens: Option<u64>,
    pub max_cost_usd: Option<f64>,
    pub compact_at_tokens: Option<u64>,
    pub compact_keep_recent: usize,

    // Policy: what the model was allowed to do at all.
    /// A denied call redirects the whole trajectory, so replaying a read-only
    /// session under `--yes` compares nothing.
    pub permission_mode: PermissionMode,
    pub trifecta: TrifectaPolicy,
    /// `none` | `bwrap` | `docker` | `landlock`. Load-bearing beyond the
    /// obvious: `shell` declares *narrower* capabilities when confined, and
    /// the interlock believes them, so the same prompt can be refused in one
    /// and allowed in the other. (`landlock` never narrows `external_send` —
    /// see the sandbox module — so it patterns with `none` for the interlock
    /// while still confining files.)
    pub sandbox: String,
    pub sandbox_network: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            mecha_version: String::new(),
            provider: String::new(),
            model: String::new(),
            workspace: PathBuf::new(),
            system_prompt: None,
            tools: Vec::new(),
            tools_hash: None,
            effort: None,
            temperature: None,
            seed: None,
            thinking: false,
            cache_prompt: false,
            max_tokens: 0,
            max_turns: 0,
            max_output_tokens: None,
            max_cost_usd: None,
            compact_at_tokens: None,
            compact_keep_recent: 0,
            permission_mode: PermissionMode::Ask,
            trifecta: TrifectaPolicy::Block,
            sandbox: "none".into(),
            sandbox_network: false,
        }
    }
}

impl RunConfig {
    /// Read it off the built agent rather than off the config file, so what is
    /// recorded is what is actually being sent — flags, layered TOML and
    /// defaults already resolved.
    pub fn of(agent: &Agent, config: &Config, provider: &str) -> Self {
        let cfg = agent.config();
        RunConfig {
            mecha_version: crate::VERSION.to_string(),
            provider: provider.to_string(),
            model: agent.model().to_string(),
            workspace: agent.ctx().workspace.clone(),
            system_prompt: agent.system().map(str::to_string),
            tools: agent
                .registry()
                .iter()
                .map(|t| t.name().to_string())
                .collect(),
            // Best-effort: recording the surface is bookkeeping beside a run,
            // and a full disk must not stop the run. A session that could not
            // record one carries no hash and reads back as `Unknown`, which is
            // exactly true rather than a silent downgrade.
            tools_hash: crate::surface::SurfaceStore::open_default()
                .and_then(|s| s.record(&agent.registry().specs()).ok()),
            effort: cfg.effort,
            temperature: config.providers.get(provider).and_then(|p| p.temperature),
            seed: config.providers.get(provider).and_then(|p| p.seed),
            thinking: cfg.thinking,
            cache_prompt: cfg.cache_prompt,
            max_tokens: cfg.max_tokens,
            max_turns: cfg.max_turns,
            max_output_tokens: cfg.max_output_tokens,
            max_cost_usd: cfg.max_cost_usd,
            compact_at_tokens: cfg.compact_at_tokens,
            compact_keep_recent: cfg.compact_keep_recent,
            permission_mode: config.tools.permission_mode,
            trifecta: config.security.trifecta,
            sandbox: config.sandbox.kind.as_str().to_string(),
            sandbox_network: config.sandbox.network,
        }
    }
}

/// How a run went, in numbers a machine can compare across sessions.
///
/// Every field is a deterministic count taken from
/// [`crate::agent::RunOutcome`] — nothing here is a model's opinion, and
/// nothing is derived from the *content* of a tool result. That is the
/// property that lets this be an input to automated grading: a counter
/// carries no instructions, so a corpus of these cannot be an injection
/// surface the way a corpus of transcript excerpts would be.
///
/// Every field defaults, so a session written before this record existed
/// loads, and a field added later does not invalidate the ones already
/// recorded.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunStats {
    #[serde(default)]
    pub turns: u32,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// False when `usage` is a lower bound rather than a measurement.
    #[serde(default)]
    pub usage_complete: bool,
    /// Why the loop stopped. The single most informative field here: it
    /// separates "the model decided it was done" from every way the harness
    /// cut it short, and none of that is visible in the answer text.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_stop_cause"
    )]
    pub stop_cause: Option<crate::agent::StopCause>,
    #[serde(default)]
    pub exhausted: bool,
    /// The model stopped of its own accord with its last call failed.
    #[serde(default)]
    pub ended_on_failed_call: bool,
    /// Tool calls attempted, and how they went. `errors` counts the
    /// environment refusing (including a call to a tool that does not exist);
    /// `denied` counts a human or a policy refusing, which is the harness
    /// working and must not be averaged in with failure.
    #[serde(default)]
    pub tool_calls: u32,
    #[serde(default)]
    pub tool_errors: u32,
    #[serde(default)]
    pub tool_denied: u32,
    #[serde(default)]
    pub tool_staged: u32,
    #[serde(default)]
    pub malformed_tool_args: u32,
    #[serde(default)]
    pub blocked_sends: u32,
    #[serde(default)]
    pub compactions: u32,
    /// Times a prompt was refused as too large and the run recovered.
    ///
    /// **`Option`, unlike every other counter here, and the difference is the
    /// point.** This field exists to be a *baseline* — the thing a change
    /// claiming to predict overflows is measured against — so the measurement
    /// spans the moment it was introduced. A row written before that knows
    /// nothing, and a plain `u32` would read it as a run that overflowed zero
    /// times, silently diluting the very rate it was added to establish.
    /// `None` says the sensor was not there. Absent is not zero, the rule
    /// [`crate::homeostat`] and [`crate::backlog`] both state at length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_overflows: Option<u32>,
    /// Times the harness told this run an approach had stopped teaching it
    /// anything (`GOAL-SYSTEM-DESIGN.md` §9.1).
    ///
    /// `Option` for `context_overflows`' reason, one field up: every threshold
    /// behind it was argued rather than measured, and this is the field that
    /// makes them answerable. A row from before the detector existed knows
    /// nothing, and reading it as a run that was never bored would dilute the
    /// rate it was added to establish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boredom_notices: Option<u32>,
    /// How many step-escalation candidates (`GOAL-SYSTEM-DESIGN.md` §5.5)
    /// actually spent a quarantined call this run. `boredom_notices`'s own
    /// reason: the pre-filter's thresholds are argued, not measured, and a
    /// row from before the mechanism existed knows nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_escalations_attempted: Option<u32>,
    /// Of those, how many came back `revise_plan`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_escalations_revised: Option<u32>,
    /// Declared post-condition checks the loop ran, and how many passed —
    /// counted off the trace by name (`step::CHECK_TRACE`), a refused check
    /// in neither. **No trace by that name is written yet** (the executor is
    /// unbuilt; see `step::CHECK_TRACE`), so a live run records `Some(0)`
    /// for both until it lands — a real zero from a real count, distinct
    /// from the `None` of a row written before the field existed. Which
    /// counter means what, once it does: `tool_calls` is the raw trace
    /// length and includes checks; `step::Work::calls` is the model's own
    /// work and excludes them; and a failed check is an `is_error` trace,
    /// so it also raises `tool_error_rate` and can set
    /// `ended_on_failed_call` — which `of_session` signs separately from
    /// `checks_passed`, so one failed check can carry two signed errors. `Option` for the same reason as `boredom_notices`: a row
    /// from before the record says nothing, and reading it as "no checks"
    /// would dilute the one structural discrepancy rate the corpus has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks_declared: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks_passed: Option<u32>,
    /// The conditions this run happened under, when the front-end asked for
    /// them. Recorded here rather than derived later because a run
    /// reconstructed against *today's* machine state is measuring the
    /// afternoon — see `GOAL-SYSTEM-DESIGN.md` §12.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homeostat: Option<crate::homeostat::Homeostat>,
    /// What had entered the conversation by the end. Recorded here as well as
    /// in [`Record::Taint`] because this record is read on its own, by a
    /// reader that is counting rather than reconstructing.
    #[serde(default)]
    pub taint: Taint,
}

impl RunStats {
    /// Fold another run's outcome in.
    ///
    /// An *episode* — a replayed session, a multi-turn eval case, a batch item
    /// — is several runs on one conversation, and is one row. Counters sum,
    /// because the episode really did spend all of it. Three fields do not,
    /// and the split is the whole reason this is a method rather than a loop
    /// at each call site:
    ///
    /// - `stop_cause`, `exhausted` and `ended_on_failed_call` describe how the
    ///   episode *ended*, so the last run wins. An episode whose first turn
    ///   ended on a failure and whose second recovered has not finished over
    ///   a failure, and summing would say it had.
    /// - `taint` merges and never resets: it is a property of the
    ///   conversation, and a later clean run does not un-read what an earlier
    ///   one read.
    /// - `usage_complete` is an AND: one lower-bound turn makes the total a
    ///   lower bound.
    pub fn absorb(&mut self, o: &crate::agent::RunOutcome) {
        self.merge(&RunStats::of_run(o));
    }

    /// Fold another *row* in, by the rules above.
    ///
    /// Same code as `absorb`, deliberately: an episode is several runs, and
    /// something has to be able to rebuild one from the rows a session
    /// recorded — `harness_probe` sizes its priority signal that way, and it
    /// has to fold exactly as the arm it will be compared against does. Two
    /// spellings of this is how a measurement arm and the thing it measures
    /// stop being comparable without anyone noticing.
    ///
    /// `homeostat`'s *conditions* are untouched, as they always were: load,
    /// memory, the backlog level and the guilt read off it belong to the run
    /// that sampled them, and an episode's several runs happened under
    /// several. The first one set keeps those. **`backlog_delta` is the
    /// exception and is summed**, because it is an act rather than a
    /// condition — what the run did to the queue — and keeping the first
    /// row's silently discarded every later run's, which for a session that
    /// parks a question is the resume that clears it (found on review: the
    /// commitment channel was reading run 1's delta as the session's).
    /// Two consequences to know before reading the two fields together:
    /// `anticipated_guilt` and `guilt_after_relief` stay the first sampling
    /// run's — the level it inherited, and that level folded with *that
    /// run's own* delta — while `backlog_delta` becomes the episode's sum,
    /// so on a resumed session neither guilt field describes the same act
    /// as the delta beside it; and a first run with no homeostat at all takes a later
    /// run's whole snapshot, so "the first run's conditions" means the
    /// first run that sampled any.
    pub fn merge(&mut self, other: &RunStats) {
        match (&mut self.homeostat, &other.homeostat) {
            (Some(mine), Some(theirs)) => {
                mine.backlog_delta = match (mine.backlog_delta, theirs.backlog_delta) {
                    (Some(a), Some(b)) => Some(a.plus(&b)),
                    (a, b) => a.or(b),
                };
            }
            (None, Some(theirs)) => self.homeostat = Some(theirs.clone()),
            _ => {}
        }
        self.turns += other.turns;
        self.usage.add(&other.usage);
        self.cost_usd = match (self.cost_usd, other.cost_usd) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.usage_complete &= other.usage_complete;
        // Last wins, `None` included. Keeping an earlier cause when the
        // final row has none would invent the one fact the field is about —
        // doctor's rule for the same value: unrecorded is unknown, never
        // assumed complete.
        self.stop_cause = other.stop_cause;
        self.exhausted = other.exhausted;
        self.ended_on_failed_call = other.ended_on_failed_call;
        self.tool_calls += other.tool_calls;
        self.tool_errors += other.tool_errors;
        self.tool_denied += other.tool_denied;
        self.tool_staged += other.tool_staged;
        self.malformed_tool_args += other.malformed_tool_args;
        self.blocked_sends += other.blocked_sends;
        self.compactions += other.compactions;
        // Summed through the `Option`, on `cost_usd`'s shape above: a live run
        // always knows its own count, so the `None` case only arises folding a
        // row read back off disk, and `or` keeps whichever arm had a sensor.
        // On `merge` rather than in `of_run` alone, so that `episode_stats` —
        // which rebuilds an episode from recorded rows — folds it too.
        self.context_overflows = match (self.context_overflows, other.context_overflows) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        // Same shape, same reason: a live run always knows its own count, and
        // omitting this arm left a session's later runs' notices silently
        // discarded — `fold` seeds from the first row and this method never
        // touched the field, so it kept whatever the first row carried
        // forever regardless of how many more rows followed.
        self.boredom_notices = match (self.boredom_notices, other.boredom_notices) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        // Same shape as boredom_notices, same reason.
        self.checks_declared = match (self.checks_declared, other.checks_declared) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.checks_passed = match (self.checks_passed, other.checks_passed) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.step_escalations_attempted = match (
            self.step_escalations_attempted,
            other.step_escalations_attempted,
        ) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.step_escalations_revised = match (
            self.step_escalations_revised,
            other.step_escalations_revised,
        ) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.taint.merge(other.taint);
    }

    /// Fold a session's recorded rows into the episode they describe.
    ///
    /// The seed is the first row rather than `default()`, because
    /// `usage_complete` is ANDed down — starting from the default's `false`
    /// would make every folded episode a lower bound.
    pub fn fold(rows: impl IntoIterator<Item = RunStats>) -> Option<RunStats> {
        let mut folded: Option<RunStats> = None;
        for row in rows {
            match &mut folded {
                Some(acc) => acc.merge(&row),
                None => folded = Some(row),
            }
        }
        folded
    }

    /// One run's outcome as a row, before any folding.
    fn of_run(o: &crate::agent::RunOutcome) -> RunStats {
        RunStats {
            turns: o.turns,
            usage: o.usage.clone(),
            cost_usd: o.cost_usd,
            usage_complete: o.usage_complete,
            stop_cause: Some(o.stop_cause),
            exhausted: o.exhausted,
            ended_on_failed_call: o.ended_on_failed_call,
            tool_calls: o.tool_calls.len() as u32,
            // `denied` is excluded, and the exclusion has to be written out: a
            // denied trace carries `is_error: true` too, so filtering on
            // `is_error` alone counts every refusal as an environment failure
            // and averages "the harness working" into the rate the candidate
            // gate and doctor both threshold on.
            tool_errors: o
                .tool_calls
                .iter()
                .filter(|c| c.unknown || (c.is_error && !c.denied))
                .count() as u32,
            tool_denied: o.tool_calls.iter().filter(|c| c.denied).count() as u32,
            tool_staged: o.tool_calls.iter().filter(|c| c.staged).count() as u32,
            malformed_tool_args: o.malformed_tool_args,
            blocked_sends: o.blocked_sends,
            compactions: o.compactions,
            // `Some`, never `None`: a live run always knows its own count, and
            // the `None` case exists only for rows written before the sensor.
            context_overflows: Some(o.context_overflows),
            boredom_notices: Some(o.boredom_notices),
            step_escalations_attempted: Some(o.step_escalations_attempted),
            step_escalations_revised: Some(o.step_escalations_revised),
            checks_declared: Some(
                o.tool_calls
                    .iter()
                    .filter(|c| {
                        c.name == crate::step::CHECK_TRACE
                            && crate::step::Outcome::of(c) != crate::step::Outcome::Refused
                    })
                    .count() as u32,
            ),
            checks_passed: Some(
                o.tool_calls
                    .iter()
                    .filter(|c| {
                        c.name == crate::step::CHECK_TRACE
                            && crate::step::Outcome::of(c) == crate::step::Outcome::Ok
                    })
                    .count() as u32,
            ),
            homeostat: o.homeostat.clone(),
            taint: o.taint,
        }
    }
}

impl From<&crate::agent::RunOutcome> for RunStats {
    fn from(o: &crate::agent::RunOutcome) -> Self {
        // `usage_complete` starts true and is ANDed down, so the default's
        // `false` would make every single-run row a lower bound.
        RunStats::of_run(o)
    }
}

/// Which front-end opened a session — the surface it was used through, not
/// what it was about.
///
/// **Written by the front-end, never inferred.** Before this field existed
/// the only way to tell a smoke test from use was the workspace path, and
/// `docs/APPRAISAL-RESEARCH.md` §1 found 46 of 143 appraised sessions were
/// development runs from a mecha checkout or a scratch directory: the
/// instrument built to measure whether the appraisal labels were degenerate
/// was measuring the harness's own test runs. A corpus reader can now ask
/// for one surface, and `Test` is excluded from every corpus readout unless
/// asked for by name.
///
/// **A closed enum written to an append-only store is a wire format**: a
/// kind this binary does not know reads as `None` ([`de_lenient_kind`]),
/// never as a failed record, and a row from before the field reads as `None`
/// too — unknown, not any particular surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    /// `mecha run`, one shot.
    Run,
    /// `mecha chat`, the readline REPL.
    Chat,
    /// `mecha tui`.
    Tui,
    /// `mecha serve`'s chat page.
    Web,
    /// The voice pipeline over `mecha serve`.
    Voice,
    /// A delegated board task (`mecha tasks work`, the web board's *ask*).
    Task,
    /// A scheduled prompt (`mecha trigger run`).
    Trigger,
    /// A front-door triage run.
    Frontdoor,
    /// A mail-driven run (`mecha mail` drafting verbs).
    Mail,
    /// A Slack thread.
    Slack,
    /// A smoke test or a development run. Set by [`SESSION_KIND_ENV`] over
    /// whatever the front-end would have written — the one override, and it
    /// only ever narrows toward this variant.
    Test,
}

/// Environment variable that marks every session a process opens as
/// [`SessionKind::Test`]. Any other value is ignored with a warning: the
/// override exists so a test harness can label its own runs, not so a
/// caller can claim a surface it is not.
pub const SESSION_KIND_ENV: &str = "MECHA_SESSION_KIND";

impl SessionKind {
    pub const ALL: [SessionKind; 11] = [
        SessionKind::Run,
        SessionKind::Chat,
        SessionKind::Tui,
        SessionKind::Web,
        SessionKind::Voice,
        SessionKind::Task,
        SessionKind::Trigger,
        SessionKind::Frontdoor,
        SessionKind::Mail,
        SessionKind::Slack,
        SessionKind::Test,
    ];

    /// The wire form — `serde`'s own `snake_case`, spelled out for callers
    /// that need a bare `&str` (a `--kind` flag, a table column).
    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Run => "run",
            SessionKind::Chat => "chat",
            SessionKind::Tui => "tui",
            SessionKind::Web => "web",
            SessionKind::Voice => "voice",
            SessionKind::Task => "task",
            SessionKind::Trigger => "trigger",
            SessionKind::Frontdoor => "frontdoor",
            SessionKind::Mail => "mail",
            SessionKind::Slack => "slack",
            SessionKind::Test => "test",
        }
    }

    /// The record half of the two parsing policies `goal::GoalRef` set: an
    /// unrecognised word is `None`, never an error.
    pub fn parse_lenient(s: &str) -> Option<Self> {
        SessionKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// [`SESSION_KIND_ENV`], read. `Some(Test)` when the variable says so,
    /// `None` otherwise — the override only ever narrows toward `Test`, so
    /// there is nothing else it could return.
    pub fn test_override() -> Option<SessionKind> {
        match std::env::var(SESSION_KIND_ENV) {
            Ok(v) if v == SessionKind::Test.as_str() => Some(SessionKind::Test),
            Ok(v) if !v.is_empty() => {
                tracing::warn!(
                    "{SESSION_KIND_ENV}={v:?} ignored: only `test` may override a session's kind"
                );
                None
            }
            _ => None,
        }
    }
}

impl std::str::FromStr for SessionKind {
    type Err = anyhow::Error;

    /// The model-facing/flag half: a bad word is an error naming the choices.
    fn from_str(s: &str) -> Result<Self> {
        SessionKind::parse_lenient(s).ok_or_else(|| {
            let choices: Vec<&str> = SessionKind::ALL.iter().map(|k| k.as_str()).collect();
            anyhow::anyhow!("unknown session kind {s:?}; one of {}", choices.join(", "))
        })
    }
}

/// Read a session kind out of a record, leniently — see [`SessionKind`].
pub fn de_lenient_kind<'de, D>(d: D) -> std::result::Result<Option<SessionKind>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    // `Value`, not `String`: a kind that is a number or an object (a newer
    // binary's shape, a hand edit) must cost the field and never the
    // record, and `Option<String>` would fail the whole `meta` row on it
    // (found on review).
    Ok(Option::<serde_json::Value>::deserialize(d)?
        .as_ref()
        .and_then(|v| v.as_str())
        .and_then(SessionKind::parse_lenient))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub provider: String,
    pub model: String,
    pub workspace: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The surface that opened this session. `None` on a row from before the
    /// field existed, or one written by a newer binary with a kind this one
    /// cannot name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_lenient_kind"
    )]
    pub kind: Option<SessionKind>,
}

pub struct Session {
    pub meta: SessionMeta,
    pub path: PathBuf,
}

/// A transcript read once: see [`Session::read`].
pub struct Transcript {
    pub meta: SessionMeta,
    pub convo: Conversation,
    /// Every `RunConfig` recorded, in order. The first is the run the session
    /// began under; a `/model` switch appends another.
    pub configs: Vec<RunConfig>,
    /// How many messages preceded each entry of `configs` in the *loaded*
    /// list — parallel to it, and what [`Transcript::config_covering`] reads.
    /// A front-end writes a `Config` at run start, before the run's own
    /// messages, so the config in effect at message `i` is the last one with
    /// a position at or below `i`. A *summarising* `Rewrite` clamps every
    /// earlier position to zero: the rewritten head's original indices are
    /// claims about a list that no longer exists, and the config in flight
    /// at the rewrite — the last of the clamped ones — is the honest answer
    /// for it (messages the rewrite kept from *earlier attaches* were
    /// genuinely recorded under older configs, but those turns are exactly
    /// the "not comparable" case `run_configs`'s own doc names, and the
    /// replay fidelity caveat is the place that says so). A *truncating*
    /// rewrite — the failed-turn rollback, whose new list is a strict prefix
    /// of the one in hand — rewrites nothing, so its positions stay exact;
    /// see the `Rewrite` arm in [`Session::read`].
    pub config_positions: Vec<usize>,
    /// Every recorded outcome, folded into the episode the session describes.
    pub episode: Option<RunStats>,
    /// The taint checkpoints, positioned against the loaded messages — the
    /// same structure [`Session::taint_timeline`] builds from a second full
    /// read, carried here because this pass already walked every record.
    /// `mecha distill` paid four complete read-and-parse passes per session
    /// (`load`, `taint_timeline`, then `for_session`'s own `read` *and*
    /// `taint_timeline`) for questions this one walk answers together.
    pub taint_timeline: TaintTimeline,
}

impl Transcript {
    /// The run config in effect at message `message_index` of the loaded
    /// list, or `None` for a transcript recorded before configs were kept.
    ///
    /// This is what a replay driver should ask, not `configs.first()`:
    /// resuming under different flags is a normal thing to do, and replaying
    /// a later attach's turns under the first attach's system prompt and
    /// tool list diverges for reasons that say nothing about those turns —
    /// which a counterfactual reader then mistakes for evidence.
    pub fn config_covering(&self, message_index: usize) -> Option<&RunConfig> {
        self.config_positions
            .iter()
            .zip(&self.configs)
            .rfind(|(pos, _)| **pos <= message_index)
            .map(|(_, cfg)| cfg)
    }
}

/// Every `Record::Outcome` in a transcript, in order.
fn outcomes_in(text: &str) -> impl Iterator<Item = RunStats> + '_ {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| match serde_json::from_str(l) {
            Ok(Record::Outcome(s)) => Some(s),
            _ => None,
        })
}

impl Session {
    /// Where transcripts live: `~/.mecha/sessions`, or `$MECHA_SESSION_DIR`.
    pub fn default_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("MECHA_SESSION_DIR") {
            return Ok(PathBuf::from(dir));
        }
        Ok(crate::work::mecha_home()?.join("sessions"))
    }

    pub fn create(dir: &Path, mut meta: SessionMeta) -> Result<Self> {
        crate::create_private_dir(dir)
            .with_context(|| format!("creating session directory {}", dir.display()))?;
        // The one override, applied where every front-end's write passes so
        // none can forget it. `None` stays `None`: an unknown surface under
        // the override is still a test, which is what the variable asserts.
        meta.kind = SessionKind::test_override().or(meta.kind);
        let path = dir.join(format!("{}.jsonl", meta.id));
        let session = Session {
            meta: meta.clone(),
            path,
        };
        session.append(&Record::Meta(meta))?;
        Ok(session)
    }

    pub fn new_id() -> String {
        // Sortable by name, and still unique when two runs start in the same
        // second.
        format!(
            "{}-{}",
            Utc::now().format("%Y%m%dT%H%M%S"),
            &uuid::Uuid::new_v4().to_string()[..8]
        )
    }

    pub fn append(&self, record: &Record) -> Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("opening {}", self.path.display()))?;
        writeln!(file, "{}", serde_json::to_string(record)?)?;
        Ok(())
    }

    pub fn append_messages(&self, messages: &[Message]) -> Result<()> {
        for m in messages {
            self.append(&Record::Message(m.clone()))?;
        }
        Ok(())
    }

    /// Record what a run did to the conversation, given the messages it
    /// started from.
    ///
    /// `before` must be what the file already holds — every front-end has
    /// appended the opening user message (and, resumed, the loaded history)
    /// before the run starts. The walk visits every state the run's rewrites
    /// replaced ([`Conversation::rewritten`]) and then the final one, so a
    /// run long enough to compact *itself* still gets its whole head into
    /// the file: each pre-rewrite snapshot extends the previous recorded
    /// state append-only (its cheap tail append), and each post-rewrite
    /// state lands as the [`Record::Rewrite`] the next transition writes.
    /// The signature takes the conversation rather than a message slice so a
    /// caller cannot record the destination while skipping the journey.
    ///
    /// [`Conversation::rewritten`]: crate::agent::Conversation
    pub fn record_run(&self, before: &[Message], convo: &Conversation) -> Result<()> {
        let mut prev: &[Message] = before;
        for state in &convo.rewritten {
            self.record_transition(prev, state)?;
            prev = state;
        }
        self.record_transition(prev, &convo.messages)
    }

    /// Record how the run went, beside what it said.
    ///
    /// Separate from [`record_run`] rather than folded into it, because the
    /// two answer to different failures: a run that errored mid-flight still
    /// has messages worth keeping and no outcome to describe, and a caller
    /// that has an outcome always has it *after* the transcript is safe.
    /// Deliberately takes the whole outcome rather than the fields, so a new
    /// counter reaches every front-end by upgrading rather than by
    /// remembering to thread it through six call sites.
    ///
    /// [`record_run`]: Session::record_run
    pub fn record_outcome(&self, outcome: &crate::agent::RunOutcome) -> Result<()> {
        self.append(&Record::Outcome(RunStats::from(outcome)))
    }

    /// Every outcome recorded in a transcript, in order, with the model and
    /// provider that were in effect when it was written.
    ///
    /// Not the session header: the TUI can switch model mid-session and
    /// records a `Config` when it does, so attributing every run to the
    /// header would credit the second model's work to the first — and defeat
    /// the per-model split in exactly the case where blending actually
    /// happens. Falls back to the header when no `Config` precedes the row,
    /// which is what an older transcript looks like.
    pub fn outcomes_attributed(path: &Path) -> Result<Vec<(String, String, RunStats)>> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut provider = String::new();
        let mut model = String::new();
        let mut out = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str(line) {
                Ok(Record::Meta(meta)) => {
                    provider = meta.provider;
                    model = meta.model;
                }
                Ok(Record::Config(cfg)) => {
                    provider = cfg.provider;
                    model = cfg.model;
                }
                Ok(Record::Outcome(stats)) => out.push((provider.clone(), model.clone(), stats)),
                _ => {}
            }
        }
        Ok(out)
    }

    /// Every outcome recorded in a transcript, in order.
    ///
    /// One per run, so a resumed session has several. Malformed lines are
    /// skipped rather than fatal, like every other reader here: a torn line
    /// is the store's problem and must not cost the rows around it.
    pub fn outcomes(path: &Path) -> Result<Vec<RunStats>> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| match serde_json::from_str(l) {
                Ok(Record::Outcome(s)) => Some(s),
                _ => None,
            })
            .collect())
    }

    /// The most recent outcome, without parsing the transcript that precedes
    /// it.
    ///
    /// [`outcomes`](Session::outcomes) reads every line because it answers
    /// "how did each run on this session go" — right for the corpus, and
    /// wrong for a display asking only where a session stands *now*. A
    /// transcript is mostly messages and an outcome is appended last, so
    /// scanning backwards finds it in one parse instead of thousands.
    ///
    /// `Ok(None)` means the transcript held no outcome at all, which is a
    /// third answer and not a failure: a run that never got as far as
    /// recording one, or a session written before the record existed.
    /// Callers must not fold it into either success or failure.
    ///
    /// Still one reader of the record format — this lives beside `outcomes`
    /// rather than in a caller, so a change to `Record` cannot leave a
    /// second, private parser behind.
    /// Every outcome a session recorded, folded into the episode it describes.
    ///
    /// `last_outcome` answers a different question — how the session *ended* —
    /// and using it as an episode's stats is a unit mismatch: a resumed chat
    /// records one row per run, while anything replaying the session drives
    /// every recorded user turn and folds all of them.
    pub fn episode_stats(path: &Path) -> Result<Option<RunStats>> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(RunStats::fold(outcomes_in(&text)))
    }

    pub fn last_outcome(path: &Path) -> Result<Option<RunStats>> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(text
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .find_map(|l| match serde_json::from_str(l) {
                Ok(Record::Outcome(s)) => Some(s),
                _ => None,
            }))
    }

    /// One before→after step. When the run only appended, the new tail is
    /// appended here too. When it rewrote what was already recorded —
    /// compaction, eviction, thinning, all of which edit earlier messages in
    /// place — a [`Record::Rewrite`] carries the whole current list instead,
    /// because slicing a rewritten transcript records a lie: the old head
    /// stays in the file, the rebuilt one (summary included) never lands.
    ///
    /// Comparison, not a flag from the loop: any mutation the loop grows
    /// later is caught by construction, and the clone this costs is one more
    /// beside the one the loop already pays per request.
    fn record_transition(&self, before: &[Message], after: &[Message]) -> Result<()> {
        let appended_only = after.len() >= before.len() && after[..before.len()] == *before;
        if appended_only {
            self.append_messages(&after[before.len()..])
        } else {
            self.append(&Record::Rewrite {
                messages: after.to_vec(),
            })
        }
    }

    /// Read a transcript back, taint included.
    ///
    /// Unparseable lines are skipped rather than failing the load — a truncated
    /// final line is the normal result of a killed process.
    pub fn load(path: &Path) -> Result<(SessionMeta, Conversation)> {
        let t = Session::read(path)?;
        Ok((t.meta, t.convo))
    }

    /// Everything a reader can want from a transcript, in **one** pass.
    ///
    /// `load`, `run_configs` and `episode_stats` each open the file and walk
    /// every line, so a caller that wants all three pays three reads and three
    /// parses of the same JSONL. That is fine for a one-off and is not fine
    /// for `harness_probe`'s pool, which considers four times the wanted
    /// episode count on every nightly — sixty-four transcripts, hundreds of KB
    /// apiece, read three times each to answer questions one walk can answer
    /// together.
    ///
    /// The three keep their own entry points, because most callers want one
    /// thing and a caller that wants one thing should not have to hold a
    /// header it has no use for. This is the seam for the caller that wants
    /// all of them.
    pub fn read(path: &Path) -> Result<Transcript> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

        let mut configs = Vec::new();
        let mut config_positions: Vec<usize> = Vec::new();
        let mut outcomes = Vec::new();
        let mut meta = None;
        let mut title = None;
        let mut messages = Vec::new();
        let mut taint = Taint::default();
        // Built here with `TaintTimeline::from_records`'s exact state
        // machine (a `Rewrite` drops checkpoints — see that function for
        // why dropping fails closed where clamping under-taints), because
        // this pass already walks every record and a second full read to
        // rebuild the same structure is the multi-read mistake this
        // function exists to end.
        let mut taint_checkpoints: Vec<(usize, Taint)> = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Record>(line).or_else(|e| lenient_record(line).ok_or(e)) {
                Ok(Record::Meta(m)) => meta = Some(m),
                // Last one wins: a conversation is renamed as it grows, and
                // the newest name is the one a person would recognise it by.
                Ok(Record::Title { title: t }) => title = Some(t),
                Ok(Record::Message(m)) => messages.push(m),
                // The conversation state as of the rewrite, wholesale. Taint
                // is deliberately not touched: summarising away the text of a
                // hostile page does not un-read it.
                //
                // Two kinds of rewrite reach this arm, and they earn opposite
                // treatment of the positions — found on review, after the
                // failed-turn rollback started writing rewrites too:
                //
                // - **A truncation** (the rolled-back failed turn: the new
                //   list is a strict prefix of the one in hand) rewrites
                //   *nothing* — every surviving message keeps its index, so
                //   every config position at or below the new length is
                //   still exact. Zeroing them here collapsed
                //   `config_covering` onto the newest attach for the whole
                //   head, which made one provider error in a resumed session
                //   reintroduce the replay-under-the-wrong-config divergence
                //   the positional lookup exists to prevent. Positions above
                //   the new length clamp to it: that config's own messages
                //   are gone, and it correctly covers only what a later turn
                //   appends (the attach is still in flight).
                // - **A summarising rewrite** (compaction, eviction) replaces
                //   the head, so old positions are claims about a list that
                //   no longer exists — clamped to zero, the config in flight
                //   covering the rewritten head, per
                //   `Transcript::config_positions`.
                //
                // Taint checkpoints drop in BOTH cases, deliberately, and
                // for a truncation that is a real (safe-direction) cost: the
                // failed turn's trailing `Record::Taint` then covers the
                // whole rolled-back list with the run's *cumulative* taint,
                // so a clean earlier turn in a session that later read a
                // hostile page and failed classifies untrusted. Over-taint,
                // never under — and keeping them would diverge from
                // `TaintTimeline::from_records`, which cannot see message
                // content to tell the two rewrites apart; provenance must
                // not depend on which reader classified it.
                Ok(Record::Rewrite { messages: m }) => {
                    // Positions survive any rewrite that leaves message `i`
                    // meaning message `i`. Three writer families produce
                    // rewrites, and only one shifts indices:
                    //
                    // - **Index-preserving, possibly content-changing**: the
                    //   in-run eviction passes (`evict_superseded_results`,
                    //   `collapse_repeated_failures`, `thin_old_results` —
                    //   all `&mut [Message]`, so length-preserving by type),
                    //   whose recorded list is *at least* as long as the
                    //   messages persisted so far because it carries the
                    //   run's unpersisted tail; and the barge-in fold (same
                    //   length, tail extended). Recognised as `m.len() >=
                    //   messages.len()` — content comparison would wrongly
                    //   fail the eviction case, whose whole point is that
                    //   content changed in place.
                    // - **A truncation** (the rollback's strict prefix):
                    //   shorter, head unchanged.
                    // - **A summarising compaction**: shorter, head
                    //   *replaced* — the one case indices genuinely die.
                    //
                    // A pathological long rewrite could masquerade as
                    // index-preserving; the bias is deliberate, because the
                    // two errors are not symmetric — misreading a
                    // summarising rewrite keeps the *old* positions (the
                    // pre-positional `configs.first()` behaviour, mildly
                    // stale), while misreading an in-place one collapses
                    // every head message onto the newest attach, the exact
                    // divergence `config_covering` exists to prevent.
                    // The shorter case compares the FULL new list, not all
                    // but its last message: the fold shape that needed the
                    // one-short comparison is length-preserving and already
                    // admitted by the `>=` arm, and under-comparing here
                    // made a compaction down to a single message vacuously
                    // "in place" (found on review — `shared == 0` compares
                    // nothing at all).
                    let in_place = m.len() >= messages.len() || messages[..m.len()] == m[..];
                    if in_place {
                        for p in &mut config_positions {
                            *p = (*p).min(m.len());
                        }
                    } else {
                        config_positions.fill(0);
                    }
                    messages = m;
                    taint_checkpoints.clear();
                }
                // Merged rather than replaced: taint only ever grows, and a
                // transcript written by an older build has none at all.
                Ok(Record::Taint(t)) => {
                    taint.merge(t);
                    taint_checkpoints.push((messages.len(), taint));
                }
                // Kept rather than discarded: this is the pass that has them
                // in hand, and the alternative is two more reads of the file
                // it just walked.
                Ok(Record::Config(c)) => {
                    config_positions.push(messages.len());
                    configs.push(c);
                }
                Ok(Record::Outcome(o)) => outcomes.push(o),
                Ok(Record::Summary { .. }) => {}
                Err(e) => tracing::warn!(error = %e, "skipping malformed transcript line"),
            }
        }

        let mut meta = meta.with_context(|| format!("{} has no session header", path.display()))?;
        // A rename recorded later in the file is the current name; the header
        // keeps the created one so `peek_meta` stays a one-line read.
        //
        // **A rename may not change what the session is.** The `task: ` /
        // `voice: ` / `web: ` prefix on the created title is read by
        // `serve::chat`'s `task_withholding`, which is how D6 — the agent may
        // not close its own task — is enforced by absence; a rename that
        // dropped it would hand a resumed delegation back `kg_task_update`.
        // Enforced here rather than trusted to every writer, because there is
        // one reader and there will be more writers.
        if let Some(t) = title.filter(|t| Self::keeps_kind(meta.title.as_deref(), t)) {
            meta.title = Some(t);
        }
        Ok(Transcript {
            meta,
            convo: Conversation::resumed(messages, taint),
            configs,
            config_positions,
            episode: RunStats::fold(outcomes),
            taint_timeline: TaintTimeline {
                checkpoints: taint_checkpoints,
            },
        })
    }

    /// Does a rename leave the session the same *kind* of session?
    ///
    /// The kind is the `<word>: ` prefix the title was created with. A rename
    /// must carry it; one that does not is dropped rather than applied, and
    /// the session keeps the name it had. Fail-closed in the direction that
    /// matters: an ignored rename costs a stale label, where an applied one
    /// can cost a withheld tool.
    ///
    /// A created title with no prefix at all constrains nothing — there is no
    /// kind to preserve, and every prefix in use today is written by this
    /// crate's own callers.
    ///
    /// Public because [`Session::read`] is not the only reader of a rename:
    /// a listing that scans for the newest `Title` without loading the
    /// transcript has to apply the same rule, or the drawer shows a name the
    /// session does not have.
    pub fn keeps_kind(created: Option<&str>, renamed: &str) -> bool {
        match created.and_then(|c| c.split_once(": ")) {
            Some((kind, _)) => renamed.starts_with(&format!("{kind}: ")),
            None => true,
        }
    }

    /// Every message the conversation ever contained, in first-seen order.
    ///
    /// `Message` records are the append-only common case. A `Rewrite` record is a
    /// compaction (or eviction, or thinning) replacing the list in place — for
    /// *loading* a session the replacement is the truth, but for a reader asking what the conversation ever held the whole
    /// point is what the replacement dropped, so its messages are unioned in
    /// rather than substituted: anything new (the summary, an edited result)
    /// joins the corpus, anything already seen is skipped. Malformed lines are
    /// skipped exactly as [`crate::session::Session::load`] skips them — a
    /// truncated final line is the normal residue of a killed process.
    pub fn messages_ever(transcript: &str) -> Vec<Message> {
        let mut seen = HashSet::new();
        let mut all = Vec::new();
        let mut admit = |m: Message, all: &mut Vec<Message>| {
            // Equality via the serialized form: `Message` is `PartialEq` but not
            // `Hash`, and the serialization is already the file's own currency.
            if let Ok(key) = serde_json::to_string(&m) {
                if seen.insert(key) {
                    all.push(m);
                }
            }
        };
        for line in transcript.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Record>(line).or_else(|e| lenient_record(line).ok_or(e)) {
                Ok(Record::Message(m)) => admit(m, &mut all),
                Ok(Record::Rewrite { messages }) => {
                    for m in messages {
                        admit(m, &mut all);
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::debug!(error = %e, "skipping malformed transcript line"),
            }
        }
        all
    }

    /// The taint checkpoints of a transcript, positioned against its messages.
    ///
    /// Every front-end appends a `Record::Taint` checkpoint *after* the
    /// messages of the run it describes, so the checkpoint that covers a
    /// message is the first one written after it — and by then the taint of
    /// everything earlier in that run, hostile fetches included, has merged
    /// in. That ordering is what makes [`TaintTimeline::covering`] safe to
    /// gate on: it can over-taint a message (a fetch later in the same run
    /// counts against it), never under-taint one.
    pub fn taint_timeline(path: &Path) -> Result<TaintTimeline> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(TaintTimeline::from_records(
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<Record>(l).ok()),
        ))
    }

    /// Every run configuration in a transcript, in the order the runs happened.
    ///
    /// A replay driver needs this per run rather than per session: resuming
    /// under different flags is a normal thing to do, and the turns before and
    /// after are not comparable. An empty result means a transcript written
    /// before this was recorded — which cannot be replayed faithfully, because
    /// the system prompt and tool list that shaped it are gone.
    pub fn run_configs(path: &Path) -> Result<Vec<RunConfig>> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| match serde_json::from_str::<Record>(l) {
                Ok(Record::Config(c)) => Some(c),
                _ => None,
            })
            .collect())
    }

    /// The header alone, without parsing the rest of the file.
    ///
    /// Listing goes through this rather than [`Session::load`] so `mecha
    /// sessions` stays O(number of sessions) instead of O(total transcript
    /// bytes) — with reflect-on-close recording every interaction, the full
    /// parse re-read the whole store to print one line per file. The header
    /// is the first record `create` writes; a file whose first record is
    /// anything else is not a session this process wrote, and is skipped
    /// exactly as `load`'s no-header error skipped it.
    pub fn peek_meta(path: &Path) -> Option<SessionMeta> {
        use std::io::BufRead;
        let file = std::fs::File::open(path).ok()?;
        let mut reader = std::io::BufReader::new(file);
        let mut first = String::new();
        loop {
            first.clear();
            if reader.read_line(&mut first).ok()? == 0 {
                return None;
            }
            if !first.trim().is_empty() {
                break;
            }
        }
        match serde_json::from_str::<Record>(&first).ok()? {
            Record::Meta(m) => Some(m),
            _ => None,
        }
    }

    /// The run summaries of a transcript, summed: total usage and turns
    /// across every run the file records. Zero for a transcript that
    /// predates the summary record or died before writing one — an honest
    /// under-count, never a guess.
    pub fn usage_totals(path: &Path) -> Result<(Usage, u32)> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut usage = Usage::default();
        let mut turns = 0u32;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(Record::Summary { usage: u, turns: t }) = serde_json::from_str(line) {
                usage.add(&u);
                turns += t;
            }
        }
        Ok((usage, turns))
    }

    /// Sessions in `dir`, newest first.
    ///
    /// A transcript whose header cannot be read is skipped so the walk
    /// stays best-effort — but skipped is not *forgotten*: callers that
    /// report on the store should use [`Session::list_counting`], because a
    /// store rotting one file at a time is otherwise invisible from every
    /// reader at once ("an unreadable store is a finding, not an empty
    /// queue" — the outbox gets `outbox_unreadable` for exactly this, and
    /// the session store got nothing).
    pub fn list(dir: &Path) -> Result<Vec<(SessionMeta, PathBuf)>> {
        Ok(Session::list_counting(dir)?.0)
    }

    /// [`Session::list`], plus how many `.jsonl` files were skipped because
    /// no header could be read from them — a torn write, a corrupt file, a
    /// permissions hole. The count is the reader's to surface; the walk
    /// itself stays best-effort either way.
    pub fn list_counting(dir: &Path) -> Result<(Vec<(SessionMeta, PathBuf)>, usize)> {
        if !dir.exists() {
            return Ok((Vec::new(), 0));
        }
        let mut out = Vec::new();
        let mut unreadable = 0usize;
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            match Session::peek_meta(&path) {
                Some(meta) => out.push((meta, path)),
                None => unreadable += 1,
            }
        }
        out.sort_by_key(|(meta, _)| std::cmp::Reverse(meta.created_at));
        Ok((out, unreadable))
    }

    /// Find a session by full id or unique prefix.
    pub fn find(dir: &Path, id_prefix: &str) -> Result<PathBuf> {
        let matches: Vec<_> = Session::list(dir)?
            .into_iter()
            .filter(|(m, _)| m.id.starts_with(id_prefix))
            .collect();
        match matches.len() {
            0 => anyhow::bail!("no session matching {id_prefix:?}"),
            1 => Ok(matches.into_iter().next().unwrap().1),
            n => anyhow::bail!("{id_prefix:?} matches {n} sessions; use a longer prefix"),
        }
    }
}

/// Where each taint checkpoint sits relative to the messages — built by
/// [`Session::taint_timeline`], consumed by provenance classification in
/// `learning`.
#[derive(Debug, Clone, Default)]
pub struct TaintTimeline {
    /// (messages recorded before this checkpoint, taint merged up to it).
    /// Merged, not raw: taint only grows, so each entry is the union of every
    /// checkpoint at or before it.
    checkpoints: Vec<(usize, Taint)>,
}

impl TaintTimeline {
    pub fn from_records(records: impl IntoIterator<Item = Record>) -> Self {
        let mut checkpoints: Vec<(usize, Taint)> = Vec::new();
        let mut messages = 0usize;
        let mut merged = Taint::default();
        for record in records {
            match record {
                Record::Message(_) => messages += 1,
                // The list was replaced, so every position recorded before it
                // is a claim about a list that no longer exists — drop them.
                // Not clamp: clamping several stale checkpoints onto the new
                // length leaves `covering` resolving to the *first* of them,
                // which is the oldest and smallest taint, and in the record
                // order the front-ends actually write (`Rewrite` then
                // `Taint`, no message between) that under-taints every
                // rewritten message — a compacting run that read a hostile
                // page would classify clean. Dropping fails the right way
                // twice over: `merged` is cumulative, so the checkpoint the
                // run writes after the rewrite carries everything the dropped
                // ones knew and covers the rewritten head with it; and a file
                // torn before that checkpoint leaves the head covered by
                // nothing, which `covering` reports as unknown — never clean.
                Record::Rewrite { messages: m } => {
                    messages = m.len();
                    checkpoints.clear();
                }
                Record::Taint(t) => {
                    merged.merge(t);
                    checkpoints.push((messages, merged));
                }
                _ => {}
            }
        }
        TaintTimeline { checkpoints }
    }

    /// The merged taint covering the message at `index`, or `None` when no
    /// checkpoint was written after it — a torn transcript, or one recorded
    /// before taint was. The caller must treat `None` as *unknown*, and
    /// unknown provenance is never clean.
    pub fn covering(&self, index: usize) -> Option<Taint> {
        self.checkpoints
            .iter()
            .find(|(n, _)| *n > index)
            .map(|(_, t)| *t)
    }
}

#[cfg(test)]
mod homeostat_record_tests {
    use super::*;
    use crate::backlog::{Backlog, BacklogDelta, Depth};
    use crate::homeostat::Homeostat;

    /// The snapshot has to reach the record, or rung 3 is a struct nothing
    /// writes. `RunStats` is what replay, the gate and the diagnostician read.
    #[test]
    fn the_conditions_a_run_happened_under_reach_its_record() {
        let bare = || crate::agent::RunOutcome {
            context_overflows: 0,
            boredom_notices: 0,
            step_escalations_attempted: 0,
            step_escalations_revised: 0,
            text: String::new(),
            stop_reason: crate::message::StopReason::EndTurn,
            usage: crate::message::Usage::default(),
            turns: 1,
            refusal: None,
            exhausted: false,
            ended_on_failed_call: false,
            tool_calls: Vec::new(),
            malformed_tool_args: 0,
            blocked_sends: 0,
            taint: crate::agent::Taint::default(),
            homeostat: None,
            stop_cause: crate::agent::StopCause::Completed,
            compactions: 0,
            usage_complete: true,
            cost_usd: None,
        };
        let mut outcome = bare();
        outcome.homeostat = Some(Homeostat {
            load_avg_1m: Some(0.56),
            backlog: Some(Backlog {
                outbox: Some(Depth {
                    waiting: 2,
                    oldest: Some("2026-08-20T09:00:00Z".into()),
                }),
                ..Backlog::default()
            }),
            backlog_delta: Some(BacklogDelta {
                outbox: Some(9),
                ..BacklogDelta::default()
            }),
            ..Homeostat::default()
        });
        let stats = RunStats::from(&outcome);
        let h = stats.homeostat.expect("recorded");
        assert_eq!(h.load_avg_1m, Some(0.56));
        assert_eq!(h.backlog_delta.unwrap().outbox, Some(9));

        // A run that did not ask for one records nothing rather than an empty
        // snapshot — absent and zero stay different all the way down.
        let unsampled = RunStats::from(&bare());
        assert_eq!(unsampled.homeostat, None);
    }
}

#[cfg(test)]
mod tests {

    /// A rename is a record, and it has to survive the file it is written to.
    ///
    /// Two things this pins, both of which fail silently otherwise: that a
    /// `Title` record *serializes at all* (this enum is internally tagged, so
    /// a newtype variant wrapping a bare `String` fails at runtime, and the
    /// only symptom is a record that never lands); and that a reader applies
    /// the last one over the header while `peek_meta` keeps reporting the
    /// created title, which is where the `web: ` / `task: ` classification
    /// comes from.
    #[test]
    fn a_rename_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("mecha-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Session::create(
            &dir,
            SessionMeta {
                id: "20260901T000000-t".into(),
                created_at: chrono::Utc::now(),
                provider: "local".into(),
                model: "first".into(),
                workspace: std::path::PathBuf::from("/tmp"),
                title: Some("web: chat-8f3a".into()),
                kind: None,
            },
        )
        .unwrap();
        s.append_messages(&[crate::message::Message::user("go")])
            .unwrap();
        s.append(&Record::Title {
            title: "web: Ostrander nomination".into(),
        })
        .unwrap();
        s.append(&Record::Title {
            title: "web: Cape Town dates".into(),
        })
        .unwrap();

        let (meta, convo) = Session::load(&s.path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("web: Cape Town dates"));
        assert_eq!(convo.messages.len(), 1, "a rename is not a message");
        assert_eq!(
            Session::peek_meta(&s.path).and_then(|m| m.title).as_deref(),
            Some("web: chat-8f3a"),
            "the header keeps the created title, so listing stays a one-line read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// D6 is enforced by the absence of a tool, and the absence is decided
    /// from the title's prefix — so a rename that changed the prefix would
    /// hand a resumed delegation back `kg_task_update`. Nothing writes such
    /// a rename today; the guard exists so that nothing can.
    #[test]
    fn a_rename_cannot_change_what_kind_of_session_this_is() {
        let dir = std::env::temp_dir().join(format!("mecha-kind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Session::create(
            &dir,
            SessionMeta {
                id: "20260902T000000-k".into(),
                created_at: chrono::Utc::now(),
                provider: "local".into(),
                model: "first".into(),
                workspace: std::path::PathBuf::from("/tmp"),
                title: Some("task: nominate someone for the Ostrander".into()),
                kind: None,
            },
        )
        .unwrap();
        s.append_messages(&[crate::message::Message::user("go")])
            .unwrap();
        // A rename that drops the kind: refused, and the session keeps the
        // name that decides its withholding.
        s.append(&Record::Title {
            title: "web: Ostrander nomination".into(),
        })
        .unwrap();
        let (meta, _) = Session::load(&s.path).unwrap();
        assert_eq!(
            meta.title.as_deref(),
            Some("task: nominate someone for the Ostrander"),
            "a rename must not be able to turn a delegation into a chat"
        );

        // A rename that keeps it: applied as normal.
        s.append(&Record::Title {
            title: "task: Ostrander nomination".into(),
        })
        .unwrap();
        let (meta, _) = Session::load(&s.path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("task: Ostrander nomination"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One walk has to answer exactly what three walks answered, or the
    /// caller that swapped to it is reading a different transcript from the
    /// one everything else reads.
    #[test]
    fn one_pass_agrees_with_the_three_readers_it_replaces() {
        let dir = std::env::temp_dir().join(format!("mecha-onepass-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Session::create(
            &dir,
            SessionMeta {
                id: "20260826T000000-x".into(),
                created_at: chrono::Utc::now(),
                provider: "local".into(),
                model: "first".into(),
                workspace: std::path::PathBuf::from("/tmp"),
                title: None,
                kind: None,
            },
        )
        .unwrap();
        s.append(&Record::Config(RunConfig {
            provider: "local".into(),
            model: "first".into(),
            ..Default::default()
        }))
        .unwrap();
        s.append_messages(&[crate::message::Message::user("go")])
            .unwrap();
        let row = |turns: u32, calls: u32| RunStats {
            turns,
            tool_calls: calls,
            usage_complete: true,
            stop_cause: Some(crate::agent::StopCause::Completed),
            ..RunStats::default()
        };
        s.append(&Record::Outcome(row(2, 3))).unwrap();
        s.append(&Record::Outcome(row(5, 7))).unwrap();

        let read = Session::read(&s.path).unwrap();
        let (meta, convo) = Session::load(&s.path).unwrap();
        assert_eq!(read.meta.id, meta.id);
        assert_eq!(read.convo.messages, convo.messages);
        assert_eq!(
            serde_json::to_string(&read.configs).unwrap(),
            serde_json::to_string(&Session::run_configs(&s.path).unwrap()).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&read.episode).unwrap(),
            serde_json::to_string(&Session::episode_stats(&s.path).unwrap()).unwrap()
        );
        // And the fold is a fold, not the last row.
        let episode = read.episode.clone().unwrap();
        assert_eq!(episode.turns, 7);
        assert_eq!(episode.tool_calls, 10);

        let _ = std::fs::remove_dir_all(&dir);
    }
    use super::*;
    use crate::message::Block;

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mecha-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn meta_with_id(id: &str) -> SessionMeta {
        SessionMeta {
            id: id.to_string(),
            created_at: Utc::now(),
            provider: "scripted".into(),
            model: "test-model".into(),
            workspace: PathBuf::from("/tmp"),
            title: None,
            kind: None,
        }
    }

    #[test]
    fn a_transcript_round_trips_its_messages_and_its_taint() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-round")).unwrap();
        session
            .append_messages(&[
                Message::user("summarise this page"),
                Message::assistant(vec![Block::text("done")]),
            ])
            .unwrap();
        session
            .append(&Record::Taint(Taint {
                private: true,
                untrusted: true,
            }))
            .unwrap();

        let (meta, convo) = Session::load(&session.path).unwrap();

        assert_eq!(meta.model, "test-model");
        assert_eq!(convo.messages.len(), 2);
        assert_eq!(convo.messages[0].text(), "summarise this page");
        assert_eq!(convo.messages[1].text(), "done");
        // The whole point of recording it: provenance cannot be recovered by
        // re-reading the content, so a resumed conversation that had read a
        // hostile page must come back with the interlock still armed.
        assert!(convo.taint.trifecta_armed());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn record_run_appends_the_tail_when_the_run_only_appended() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-tail")).unwrap();
        let before = vec![Message::user("go")];
        session.append_messages(&before).unwrap();

        let mut after = Conversation::from(before.clone());
        after.push(Message::assistant(vec![Block::text("done")]));
        session.record_run(&before, &after).unwrap();

        let (_, convo) = Session::load(&session.path).unwrap();
        assert_eq!(convo.messages.len(), 2);
        assert_eq!(convo.messages[1].text(), "done");
        // And no rewrite record for the ordinary case: the file stays a plain
        // append log unless the run actually rewrote history.
        let text = std::fs::read_to_string(&session.path).unwrap();
        assert!(!text.contains("\"record\":\"rewrite\""), "{text}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn record_run_records_a_rewrite_when_compaction_touched_the_head() {
        // The regression this pins, from a 2026-08-07 benchmark transcript:
        // a compacted run recorded via the append-only slice kept the stale
        // head and skipped the rebuilt one, so the file held 8 assistant
        // turns of a 28-turn run, beginning mid-conversation, with no sign a
        // compaction had happened. Resuming that transcript resumes a
        // conversation the run never had.
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-rw")).unwrap();
        let before = vec![Message::user("go")];
        session.append_messages(&before).unwrap();

        // What compaction leaves behind: the head rewritten in place
        // (instruction plus summary), then the surviving tail.
        let mut head = before[0].clone();
        head.content
            .push(Block::text("[Earlier turns were compacted]"));
        let after = Conversation::from(vec![head, Message::assistant(vec![Block::text("done")])]);
        session.record_run(&before, &after).unwrap();

        let (_, convo) = Session::load(&session.path).unwrap();
        assert_eq!(convo.messages.len(), 2);
        assert!(
            convo.messages[0].text().contains("compacted"),
            "the rebuilt head must be what loads: {:?}",
            convo.messages[0].text()
        );
        assert_eq!(convo.messages[1].text(), "done");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The gap this closes: a run long enough to compact *itself* produced
    /// turns the file never saw — the front-end records at run end, and the
    /// rewrite record carries only what survived. With the pre-rewrite states
    /// walked first, the dropped turn is in the file (where `recall` searches
    /// the union) while `load` still returns only the final state.
    #[test]
    fn record_run_walks_the_states_a_mid_run_rewrite_replaced() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-midrun")).unwrap();
        let before = vec![Message::user("go")];
        session.append_messages(&before).unwrap();

        // The state the run reached before compaction: the opening message
        // plus a turn holding the detail the summary will drop.
        let mut reached = before.clone();
        reached.push(Message::assistant(vec![Block::text(
            "the magic number is 74656",
        )]));
        // What compaction left, then one more turn on top of it.
        let compacted = vec![
            Message::user("[summary: a number was computed]"),
            Message::assistant(vec![Block::text("done")]),
        ];
        let mut convo = Conversation::from(compacted);
        convo.rewritten = vec![reached];

        session.record_run(&before, &convo).unwrap();

        // Loading replays to the final state — the summary, not the head.
        let (_, loaded) = Session::load(&session.path).unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert!(loaded.messages[0].text().contains("summary"));

        // And the dropped detail is in the file all the same, which is what
        // recall's union over every recorded message reads back.
        let text = std::fs::read_to_string(&session.path).unwrap();
        assert!(
            text.contains("74656"),
            "the pre-rewrite turn never reached the file: {text}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_rewrite_drops_stale_taint_positions_instead_of_shadowing_later_ones() {
        // The record order the front-ends actually write, across two runs of
        // one chat session: run 1's messages and its clean checkpoint, then
        // run 2 compacts (a rewrite, shrinking the list) after reading a
        // hostile page, and checkpoints — `Rewrite` then `Taint`, with no
        // message record between. A stale checkpoint kept in any form sits
        // at-or-before the new length, and `covering` takes the *first*
        // checkpoint past an index, so keeping it hands every rewritten
        // message the older, clean taint — under-tainting, the one direction
        // the timeline must never be wrong in.
        let msg = || Message::user("m");
        let mut records: Vec<Record> = (0..10).map(|_| Record::Message(msg())).collect();
        records.push(Record::Taint(Taint {
            private: true,
            untrusted: false,
        }));
        records.push(Record::Rewrite {
            messages: vec![msg(), msg()],
        });
        records.push(Record::Taint(Taint {
            private: true,
            untrusted: true,
        }));

        let timeline = TaintTimeline::from_records(records);
        // Every position in the rewritten list is covered by the post-rewrite
        // checkpoint, which merged the dropped one's taint — over-taint,
        // never under.
        for index in 0..2 {
            let covering = timeline.covering(index).expect("a checkpoint covers it");
            assert!(
                covering.untrusted,
                "message {index} classified by a stale pre-rewrite checkpoint"
            );
            assert!(covering.private, "the dropped checkpoint's taint was lost");
        }
    }

    #[test]
    fn a_transcript_torn_after_a_rewrite_reports_unknown_not_clean() {
        // The process died between writing the rewrite and its taint
        // checkpoint. Nothing covers the rewritten messages, and `covering`
        // must say so — the learning classifier treats unknown as untrusted,
        // and a clean answer here would be the laundering path.
        let msg = || Message::user("m");
        let records = vec![
            Record::Message(msg()),
            Record::Taint(Taint {
                private: true,
                untrusted: true,
            }),
            Record::Rewrite {
                messages: vec![msg(), msg()],
            },
        ];
        let timeline = TaintTimeline::from_records(records);
        assert_eq!(timeline.covering(0), None);
        assert_eq!(timeline.covering(1), None);
    }

    #[test]
    fn taint_records_merge_so_a_later_clean_one_cannot_disarm_the_interlock() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-merge")).unwrap();

        // The order a real run writes them in: one leg arrives, then the other,
        // and the loop may checkpoint again with nothing new to say.
        session
            .append(&Record::Taint(Taint {
                untrusted: true,
                private: false,
            }))
            .unwrap();
        session
            .append(&Record::Taint(Taint {
                private: true,
                untrusted: false,
            }))
            .unwrap();
        session.append(&Record::Taint(Taint::default())).unwrap();

        let (_, convo) = Session::load(&session.path).unwrap();

        // Replacing rather than merging would leave this clean, and resuming
        // would hand the model the attacker's page with the guard switched off.
        assert!(convo.taint.private, "an earlier private leg was dropped");
        assert!(
            convo.taint.untrusted,
            "an earlier untrusted leg was dropped"
        );
        assert!(convo.taint.trifecta_armed());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_transcript_written_before_taint_was_recorded_loads_clean() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-old")).unwrap();
        session.append_messages(&[Message::user("hello")]).unwrap();

        let (_, convo) = Session::load(&session.path).unwrap();

        assert_eq!(convo.messages.len(), 1);
        assert!(!convo.taint.private);
        assert!(!convo.taint.untrusted);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_truncated_final_line_does_not_lose_the_rest_of_the_transcript() {
        use std::io::Write;
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-killed")).unwrap();
        session.append_messages(&[Message::user("first")]).unwrap();
        session
            .append(&Record::Taint(Taint {
                private: true,
                untrusted: false,
            }))
            .unwrap();

        // What a killed process leaves behind: a half-written final record.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&session.path)
            .unwrap();
        write!(file, "{{\"record\":\"message\",\"role\":\"assis").unwrap();
        drop(file);

        let (_, convo) = Session::load(&session.path).unwrap();

        assert_eq!(convo.messages.len(), 1);
        assert_eq!(convo.messages[0].text(), "first");
        assert!(
            convo.taint.private,
            "a torn last line lost the taint before it"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_configs_come_back_in_order_one_per_attach() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-cfg")).unwrap();

        // What a resume under different flags looks like on disk.
        let first = RunConfig {
            compact_at_tokens: None,
            ..RunConfig::default()
        };
        let second = RunConfig {
            compact_at_tokens: Some(1200),
            ..RunConfig::default()
        };
        session.append(&Record::Config(first)).unwrap();
        session
            .append_messages(&[Message::user("first run")])
            .unwrap();
        session.append(&Record::Config(second)).unwrap();

        let configs = Session::run_configs(&session.path).unwrap();

        assert_eq!(configs.len(), 2, "one record per attach, in order");
        assert_eq!(configs[0].compact_at_tokens, None);
        // The turns before and after are not comparable, and only a per-attach
        // record can say where the line is.
        assert_eq!(configs[1].compact_at_tokens, Some(1200));

        // And the messages still load, unbothered by the new record type.
        let (_, convo) = Session::load(&session.path).unwrap();
        assert_eq!(convo.messages.len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The replay driver's question, answered positionally: which config was
    /// in effect *at this message* — not `first()`, which replayed a resumed
    /// session's later attach under the first attach's system prompt and
    /// tool list, diverging for reasons that said nothing about the turn
    /// being probed.
    #[test]
    fn config_covering_names_the_attach_a_message_actually_ran_under() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-cover")).unwrap();
        let first = RunConfig::default();
        let second = RunConfig {
            compact_at_tokens: Some(1200),
            ..RunConfig::default()
        };
        session.append(&Record::Config(first)).unwrap();
        session
            .append_messages(&[
                Message::user("first attach"),
                Message::assistant(vec![Block::text("done")]),
            ])
            .unwrap();
        session.append(&Record::Config(second)).unwrap();
        session
            .append_messages(&[Message::user("second attach")])
            .unwrap();

        let t = Session::read(&session.path).unwrap();
        assert_eq!(
            t.config_covering(0).unwrap().compact_at_tokens,
            None,
            "message 0 ran under the first attach"
        );
        assert_eq!(
            t.config_covering(2).unwrap().compact_at_tokens,
            Some(1200),
            "message 2 ran under the second attach"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The failed-turn rollback writes a *truncating* rewrite — the new list
    /// is a strict prefix of the recorded one, nothing rewritten — and the
    /// review found the zero-clamp collapsing it anyway: one provider error
    /// in a resumed session made every head message report the newest
    /// attach's config, reintroducing the replay-under-the-wrong-config
    /// divergence the positional lookup exists to prevent. A truncation
    /// keeps its positions exact.
    #[test]
    fn a_truncating_rewrite_keeps_config_positions_exact() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-trunc")).unwrap();
        let a = RunConfig::default();
        let b = RunConfig {
            compact_at_tokens: Some(1200),
            ..RunConfig::default()
        };
        // Attach A: messages 0-1. Attach B: message 2, a user turn whose run
        // then fails.
        session.append(&Record::Config(a)).unwrap();
        session
            .append_messages(&[
                Message::user("first attach"),
                Message::assistant(vec![Block::text("done")]),
            ])
            .unwrap();
        session.append(&Record::Config(b)).unwrap();
        session
            .append_messages(&[Message::user("the turn that fails")])
            .unwrap();
        // The failed-turn rollback, exactly as every error arm now runs it:
        // restore-then-pop, then record the rolled-back state — a strict
        // prefix, which record_run expresses as a truncating Rewrite.
        let before = Session::load(&session.path).unwrap().1.messages;
        let mut convo = crate::agent::Conversation::from(before.clone());
        convo.roll_back_failed_turn(before.clone());
        session.record_run(&before, &convo).unwrap();

        let t = Session::read(&session.path).unwrap();
        assert_eq!(
            t.config_covering(0).unwrap().compact_at_tokens,
            None,
            "message 0 ran under attach A and must still say so after the rollback"
        );
        assert_eq!(
            t.config_covering(1).unwrap().compact_at_tokens,
            None,
            "message 1 likewise"
        );
        // A turn appended after the rollback runs under the attach still in
        // flight — B.
        assert_eq!(
            t.config_covering(2).unwrap().compact_at_tokens,
            Some(1200),
            "the next appended turn is attach B's"
        );

        // And a *fold* rewrite — same length, only the tail's content
        // extended, which is what a barge-in submit writes — preserves
        // positions the same way: message 0 still ran under attach A.
        let mut folded = Session::load(&session.path).unwrap().1.messages;
        let barged = Message::user("the turn that barged in");
        session.append(&Record::Message(barged.clone())).unwrap();
        folded.push(barged);
        crate::agent::append_user_text(&mut folded, "and another thing".into());
        session
            .append(&Record::Rewrite {
                messages: folded.clone(),
            })
            .unwrap();
        let t = Session::read(&session.path).unwrap();
        assert_eq!(
            t.config_covering(0).unwrap().compact_at_tokens,
            None,
            "a fold rewrite must not collapse the head onto the newest attach"
        );

        // An eviction-shaped rewrite — content changed *in place* (a
        // superseded result stubbed out), length at least the persisted
        // list's — preserves positions too: it changes what a message says,
        // never which message it is. Found on review: the head-equality
        // check wrongly failed this exact case, whose whole point is that
        // content differs.
        let mut evicted = folded.clone();
        evicted[1] = Message::assistant(vec![Block::text("[superseded]")]);
        session
            .append(&Record::Rewrite {
                messages: evicted.clone(),
            })
            .unwrap();
        let t = Session::read(&session.path).unwrap();
        assert_eq!(
            t.config_covering(0).unwrap().compact_at_tokens,
            None,
            "an in-place eviction must not collapse the head onto the newest attach"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A *summarising* rewrite replaces the list, so positions recorded
    /// against the old one are claims about a list that no longer exists —
    /// they clamp to zero, and the config in flight at the rewrite (the
    /// last of them) covers the rewritten head. This is the `fill(0)`
    /// branch's own coverage (found missing on review: the truncation test
    /// exercises only the in-place branch), so the fixture carries two
    /// configs and asserts the head resolves to the *newest*. A transcript
    /// with no configs at all answers `None`, never a default.
    #[test]
    fn config_covering_survives_a_rewrite_and_answers_none_without_configs() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-coverrw")).unwrap();
        let a = RunConfig::default();
        let b = RunConfig {
            compact_at_tokens: Some(1200),
            ..RunConfig::default()
        };
        session.append(&Record::Config(a)).unwrap();
        session
            .append_messages(&[
                Message::user("a long history"),
                Message::assistant(vec![Block::text("...")]),
            ])
            .unwrap();
        session.append(&Record::Config(b)).unwrap();
        session.append_messages(&[Message::user("more")]).unwrap();
        session
            .append(&Record::Rewrite {
                messages: vec![Message::user("[summary]")],
            })
            .unwrap();

        let t = Session::read(&session.path).unwrap();
        assert_eq!(
            t.config_covering(0).unwrap().compact_at_tokens,
            Some(1200),
            "a summarising rewrite's head resolves to the config in flight — \
             the newest of the clamped ones, not the first attach's"
        );

        let bare = Session::create(&dir, meta_with_id("20260101T000001-bare")).unwrap();
        bare.append_messages(&[Message::user("hello")]).unwrap();
        assert!(Session::read(&bare.path)
            .unwrap()
            .config_covering(0)
            .is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_transcript_recorded_before_this_existed_reports_no_configs() {
        // Not an error: it is the honest answer, and it is what tells a replay
        // driver the recording cannot be reproduced faithfully.
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-legacy")).unwrap();
        session.append_messages(&[Message::user("hello")]).unwrap();

        assert!(Session::run_configs(&session.path).unwrap().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_taint_timeline_covers_each_message_with_its_runs_checkpoint() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-tl")).unwrap();

        // Run one: clean. Its checkpoint lands after its messages.
        session
            .append_messages(&[Message::user("list the files")])
            .unwrap();
        session
            .append_messages(&[Message::assistant(vec![Block::text("done")])])
            .unwrap();
        session.append(&Record::Taint(Taint::default())).unwrap();
        // Run two: a hostile page enters; the checkpoint records it.
        session
            .append_messages(&[Message::user("fetch that page")])
            .unwrap();
        session
            .append_messages(&[Message::assistant(vec![Block::text("fetched")])])
            .unwrap();
        session
            .append(&Record::Taint(Taint {
                untrusted: true,
                private: false,
            }))
            .unwrap();

        let tl = Session::taint_timeline(&session.path).unwrap();

        // Messages 0–1 are covered by the clean checkpoint...
        assert!(!tl.covering(0).unwrap().untrusted);
        assert!(!tl.covering(1).unwrap().untrusted);
        // ...2–3 by the armed one. Over-tainting within a run is the safe
        // direction: a fetch later in the same run counts against a message
        // before it, never the reverse.
        assert!(tl.covering(2).unwrap().untrusted);
        assert!(tl.covering(3).unwrap().untrusted);
        // Beyond the last checkpoint is unknown, and unknown is the caller's
        // cue to fail closed.
        assert_eq!(tl.covering(4).map(|t| t.untrusted), None);

        // `Session::read` builds the same timeline in its one pass — the
        // two must never disagree, or the single-read callers (`distill`,
        // the closure appraisal) classify provenance differently from the
        // dedicated reader.
        let carried = Session::read(&session.path).unwrap().taint_timeline;
        for i in 0..5 {
            assert_eq!(
                carried.covering(i).map(|t| t.untrusted),
                tl.covering(i).map(|t| t.untrusted),
                "read()'s carried timeline diverged from taint_timeline() at {i}"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_pre_taint_transcript_has_an_empty_timeline() {
        // Sessions recorded before taint existed can establish nothing, so
        // every position must come back None — which classification turns
        // into Untrusted, never Clean.
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-notl")).unwrap();
        session.append_messages(&[Message::user("hello")]).unwrap();

        let tl = Session::taint_timeline(&session.path).unwrap();
        assert!(tl.covering(0).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn listing_reads_only_the_first_record_and_skips_files_without_a_header() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-peek")).unwrap();
        session.append_messages(&[Message::user("hello")]).unwrap();

        // A stray JSONL file whose first record is not a header is skipped —
        // the contract is now explicitly "the header is the first record",
        // which is where `create` writes it; buried headers no longer count,
        // and that is the price of listing without parsing every transcript.
        let stray = serde_json::to_string(&Record::Message(Message::user("orphan"))).unwrap();
        let meta = serde_json::to_string(&Record::Meta(meta_with_id("buried"))).unwrap();
        std::fs::write(dir.join("stray.jsonl"), format!("{stray}\n{meta}\n")).unwrap();

        let listed = Session::list(&dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0.id, "20260101T000000-peek");

        // Skipped is not forgotten: the counting variant reports the same
        // sessions plus how many files it had to skip, so a reporting
        // caller can surface the rot the best-effort walk steps over.
        let (counted, unreadable) = Session::list_counting(&dir).unwrap();
        assert_eq!(counted.len(), 1);
        assert_eq!(unreadable, 1);

        // And the peek agrees with the full load about what the header says.
        let peeked = Session::peek_meta(&session.path).unwrap();
        let (loaded, _) = Session::load(&session.path).unwrap();
        assert_eq!(peeked.id, loaded.id);
        assert_eq!(peeked.model, loaded.model);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_outcome_record_survives_a_round_trip_and_does_not_disturb_the_transcript() {
        use crate::agent::{RunOutcome, StopCause, ToolCallTrace};
        use crate::message::StopReason;

        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-outcome")).unwrap();
        session
            .append_messages(&[Message::user("go"), Message::assistant(vec![])])
            .unwrap();

        let call = |is_error: bool, denied: bool, unknown: bool, staged: bool| ToolCallTrace {
            name: "fs_edit".into(),
            input: serde_json::json!({}),
            is_error,
            denied,
            unknown,
            staged,
        };
        let outcome = RunOutcome {
            homeostat: None,
            context_overflows: 0,
            boredom_notices: 0,
            step_escalations_attempted: 0,
            step_escalations_revised: 0,
            text: "done".into(),
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 4,
                ..Usage::default()
            },
            turns: 3,
            refusal: None,
            exhausted: false,
            ended_on_failed_call: true,
            tool_calls: vec![
                call(true, false, false, false),
                call(false, false, true, false),
                call(false, true, false, false),
                call(false, false, false, true),
            ],
            malformed_tool_args: 1,
            blocked_sends: 2,
            taint: Taint {
                private: true,
                untrusted: false,
            },
            stop_cause: StopCause::Completed,
            compactions: 4,
            cost_usd: Some(0.5),
            usage_complete: true,
        };
        session.record_outcome(&outcome).unwrap();

        let stats = Session::outcomes(&session.path).unwrap();
        assert_eq!(stats.len(), 1);
        let got = &stats[0];
        assert_eq!(got.turns, 3);
        assert_eq!(got.stop_cause, Some(StopCause::Completed));
        assert!(got.ended_on_failed_call);
        assert_eq!(got.tool_calls, 4);
        // The environment refusing: the error and the unknown tool. A denial
        // is the harness working and must never be averaged in with failure.
        assert_eq!(got.tool_errors, 2);
        assert_eq!(got.tool_denied, 1);
        assert_eq!(got.tool_staged, 1);
        assert_eq!(got.malformed_tool_args, 1);
        assert_eq!(got.blocked_sends, 2);
        assert_eq!(got.compactions, 4);
        assert!(got.taint.private && !got.taint.untrusted);

        // And it is inert to every existing reader: the record is not a
        // message, so the conversation is unchanged, and `usage_totals`
        // counts `Summary` records only.
        let (_, convo) = Session::load(&session.path).unwrap();
        assert_eq!(convo.messages.len(), 2);
        assert_eq!(Session::usage_totals(&session.path).unwrap().1, 0);
    }

    #[test]
    fn an_episode_of_several_runs_sums_its_costs_and_takes_its_ending_from_the_last() {
        use crate::agent::{RunOutcome, StopCause, ToolCallTrace};
        use crate::message::StopReason;

        let outcome =
            |turns: u32, calls: usize, errored: bool, ended_failed: bool, cause| RunOutcome {
                homeostat: None,
                context_overflows: 0,
                boredom_notices: 0,
                step_escalations_attempted: 0,
                step_escalations_revised: 0,
                text: String::new(),
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 1,
                    ..Usage::default()
                },
                turns,
                refusal: None,
                exhausted: false,
                ended_on_failed_call: ended_failed,
                tool_calls: (0..calls)
                    .map(|_| ToolCallTrace {
                        name: "fs_edit".into(),
                        input: serde_json::json!({}),
                        is_error: errored,
                        denied: false,
                        unknown: false,
                        staged: false,
                    })
                    .collect(),
                malformed_tool_args: 1,
                blocked_sends: 0,
                taint: Taint {
                    private: true,
                    untrusted: false,
                },
                stop_cause: cause,
                compactions: 1,
                cost_usd: Some(0.25),
                usage_complete: true,
            };

        let mut stats = RunStats {
            usage_complete: true,
            ..RunStats::default()
        };
        // Turn one fails and ends over the failure; turn two recovers.
        stats.absorb(&outcome(2, 3, true, true, StopCause::MaxTurns));
        stats.absorb(&outcome(4, 5, false, false, StopCause::Completed));

        // Costs sum: the episode really did spend all of it.
        assert_eq!(stats.turns, 6);
        assert_eq!(stats.tool_calls, 8);
        assert_eq!(stats.tool_errors, 3);
        assert_eq!(stats.malformed_tool_args, 2);
        assert_eq!(stats.compactions, 2);
        assert_eq!(stats.cost_usd, Some(0.5));
        assert_eq!(stats.usage.input_tokens, 20);

        // The ending is the last run's. An episode whose first turn ended on
        // a failure and whose second recovered has not finished over one.
        assert_eq!(stats.stop_cause, Some(StopCause::Completed));
        assert!(!stats.ended_on_failed_call);

        // Taint merges and never resets: a later clean run does not un-read
        // what an earlier one read.
        assert!(stats.taint.private);
    }

    /// `merge` had no arm for this field at all, so `fold`'s first-row seed
    /// kept whatever the first run recorded and every later run's notices
    /// were silently dropped — diluting the exact rate the sensor exists to
    /// establish, in the direction `context_overflows`' own `Option` is
    /// there to prevent.
    #[test]
    fn boredom_notices_sum_across_an_episodes_runs_like_context_overflows() {
        let mut stats = RunStats {
            boredom_notices: Some(2),
            ..RunStats::default()
        };
        stats.merge(&RunStats {
            boredom_notices: Some(3),
            ..RunStats::default()
        });
        assert_eq!(stats.boredom_notices, Some(5));

        // `None` behaves like `context_overflows`: a live run always knows
        // its own count, so `None` only arises from a pre-sensor row, and
        // `or` keeps whichever side had a sensor rather than treating the
        // other's silence as zero.
        let mut unsampled = RunStats {
            boredom_notices: None,
            ..RunStats::default()
        };
        unsampled.merge(&RunStats {
            boredom_notices: Some(1),
            ..RunStats::default()
        });
        assert_eq!(unsampled.boredom_notices, Some(1));
    }

    #[test]
    fn one_lower_bound_turn_makes_the_whole_episode_a_lower_bound() {
        use crate::agent::{RunOutcome, StopCause};
        use crate::message::StopReason;

        let mut incomplete = RunOutcome {
            homeostat: None,
            context_overflows: 0,
            boredom_notices: 0,
            step_escalations_attempted: 0,
            step_escalations_revised: 0,
            text: String::new(),
            stop_reason: StopReason::Other,
            usage: Usage::default(),
            turns: 1,
            refusal: None,
            exhausted: true,
            ended_on_failed_call: false,
            tool_calls: Vec::new(),
            malformed_tool_args: 0,
            blocked_sends: 0,
            taint: Taint::default(),
            stop_cause: StopCause::Interrupted,
            compactions: 0,
            cost_usd: None,
            usage_complete: false,
        };

        let mut stats = RunStats {
            usage_complete: true,
            ..RunStats::default()
        };
        stats.absorb(&incomplete);
        assert!(!stats.usage_complete);

        // And it stays false: a later complete turn cannot repair a total
        // that already lost a measurement.
        incomplete.usage_complete = true;
        stats.absorb(&incomplete);
        assert!(!stats.usage_complete);
    }

    #[test]
    fn a_transcript_with_no_outcome_records_reads_as_empty_not_as_an_error() {
        // Sessions written before this record existed, and runs that died
        // before producing an outcome. Unknown is not zero-with-confidence,
        // but it must not be a failure either.
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-no-outcome")).unwrap();
        session.append_messages(&[Message::user("go")]).unwrap();
        assert!(Session::outcomes(&session.path).unwrap().is_empty());
    }

    #[test]
    fn usage_totals_sum_every_run_and_report_zero_for_a_summaryless_file() {
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-usage")).unwrap();

        // No summary yet — a run that died mid-flight. Zero, not an error.
        assert_eq!(Session::usage_totals(&session.path).unwrap().1, 0);

        // Two runs on one session (chat, resume): the totals are the sum.
        session
            .append(&Record::Summary {
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 10,
                    ..Default::default()
                },
                turns: 2,
            })
            .unwrap();
        session
            .append(&Record::Summary {
                usage: Usage {
                    input_tokens: 50,
                    output_tokens: 5,
                    ..Default::default()
                },
                turns: 1,
            })
            .unwrap();

        let (usage, turns) = Session::usage_totals(&session.path).unwrap();
        assert_eq!(usage.input_tokens, 150);
        assert_eq!(usage.output_tokens, 15);
        assert_eq!(turns, 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn the_session_directory_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        // A fresh path, so `create` makes the directory itself.
        let dir = std::env::temp_dir().join(format!("mecha-session-{}", uuid::Uuid::new_v4()));
        Session::create(&dir, meta_with_id("20260101T000000-perms")).unwrap();

        // Transcripts hold whatever the tools returned — mail bodies
        // included — so the directory gets the token-file rule.
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The same leniency for a `rewrite` record, which carries a whole message
    /// list and is the record where dropping the line costs a compaction: the
    /// resumed conversation would be the pre-compaction one, oversized, with
    /// the summary gone.
    #[test]
    fn a_rewrite_from_a_newer_build_is_applied_minus_what_this_one_cannot_read() {
        use std::io::Write;
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-rewrite")).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&session.path)
            .unwrap();
        writeln!(
            file,
            r#"{{"record":"message","role":"user","content":[{{"type":"text","text":"before"}}]}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"record":"message","role":"assistant","content":[{{"type":"text","text":"long ago"}}]}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"record":"rewrite","messages":[{{"role":"user","content":[{{"type":"text","text":"compacted head"}},{{"type":"hologram","frames":2}}]}}]}}"#
        )
        .unwrap();
        drop(file);

        let t = Session::read(&session.path).unwrap();
        assert_eq!(
            t.convo.messages.len(),
            1,
            "the rewrite replaced the list: {:?}",
            t.convo.messages
        );
        assert!(t.convo.messages[0].text().contains("compacted head"));
        assert_eq!(
            t.convo.messages[0].content.len(),
            1,
            "the unknown block is dropped"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A closed enum in an append-only store is a wire format. A newer build
    /// writing a block kind or a stop cause this one does not know must cost
    /// the field, never the record: the message was dropped from the resumed
    /// conversation (orphaning any `tool_result` beside it) and the outcome
    /// vanished from the corpus.
    #[test]
    fn records_from_a_newer_build_degrade_to_what_this_one_can_read() {
        use std::io::Write;
        let dir = tmpdir();
        let session = Session::create(&dir, meta_with_id("20260101T000000-newer")).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&session.path)
            .unwrap();
        // An assistant turn carrying a block kind from the future beside one
        // we know, then the tool result that answers the known block.
        writeln!(
            file,
            r#"{{"record":"message","role":"assistant","content":[{{"type":"hologram","frames":3}},{{"type":"tool_use","id":"t1","name":"echo","input":{{}}}}]}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"record":"message","role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"ok","is_error":false}}]}}"#
        )
        .unwrap();
        // An outcome whose stop cause this build has never heard of.
        writeln!(
            file,
            r#"{{"record":"outcome","turns":2,"stop_cause":"from_the_future","exhausted":false}}"#
        )
        .unwrap();
        // A message made only of unknown blocks has nothing to keep.
        writeln!(
            file,
            r#"{{"record":"message","role":"assistant","content":[{{"type":"hologram","frames":1}}]}}"#
        )
        .unwrap();
        drop(file);

        let t = Session::read(&session.path).unwrap();
        let messages = &t.convo.messages;
        assert_eq!(
            messages.len(),
            2,
            "the readable message and its result survive"
        );
        assert_eq!(
            messages[0].content.len(),
            1,
            "only the unknown block is dropped: {:?}",
            messages[0]
        );
        assert!(
            crate::compact::orphaned_tool_results(messages).is_empty(),
            "keeping the message is what keeps the pairing"
        );
        let episode = t.episode.expect("the outcome record survives");
        assert!(
            episode.stop_cause.is_none(),
            "an unknown cause is None, not a lost record"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_transcript_with_no_header_is_refused() {
        let dir = tmpdir();
        let path = dir.join("headerless.jsonl");
        let line = serde_json::to_string(&Record::Message(Message::user("orphan"))).unwrap();
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let err = Session::load(&path).unwrap_err().to_string();
        assert!(err.contains("no session header"), "unexpected error: {err}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_ambiguous_id_prefix_is_an_error_rather_than_a_guess() {
        let dir = tmpdir();
        Session::create(&dir, meta_with_id("20260101T000000-aaaaaaaa")).unwrap();
        Session::create(&dir, meta_with_id("20260101T000000-bbbbbbbb")).unwrap();

        let err = Session::find(&dir, "20260101").unwrap_err().to_string();
        assert!(
            err.contains("matches 2 sessions"),
            "unexpected error: {err}"
        );

        // A full id still resolves, and resuming the wrong transcript is the
        // failure being guarded against.
        let path = Session::find(&dir, "20260101T000000-aaaaaaaa").unwrap();
        assert!(path.ends_with("20260101T000000-aaaaaaaa.jsonl"));

        assert!(Session::find(&dir, "nothing-like-this").is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    // --- session kind -------------------------------------------------------

    fn kind_of(path: &Path) -> Option<SessionKind> {
        Session::read(path).unwrap().meta.kind
    }

    #[test]
    fn a_meta_row_from_before_kinds_existed_reads_as_unknown_not_any_surface() {
        let row = r#"{"id":"20260101T000000-old","created_at":"2026-01-01T00:00:00Z","provider":"local","model":"m","workspace":"/tmp"}"#;
        let meta: SessionMeta = serde_json::from_str(row).unwrap();
        assert_eq!(meta.kind, None);
    }

    #[test]
    fn a_kind_from_a_newer_build_degrades_to_unknown_rather_than_failing_the_record() {
        let row = r#"{"id":"20260101T000000-new","created_at":"2026-01-01T00:00:00Z","provider":"local","model":"m","workspace":"/tmp","kind":"hologram"}"#;
        let meta: SessionMeta = serde_json::from_str(row).unwrap();
        assert_eq!(meta.kind, None, "unknown, never an error");
        // And a kind of the wrong shape entirely — not merely an unknown
        // word — costs the field, never the record (found on review).
        for bad in ["7", r#"{"surface":"web"}"#, r#"["web"]"#, "true"] {
            let row = format!(
                r#"{{"id":"x","created_at":"2026-01-01T00:00:00Z","provider":"local","model":"m","workspace":"/tmp","kind":{bad}}}"#
            );
            let meta: SessionMeta =
                serde_json::from_str(&row).unwrap_or_else(|e| panic!("{bad}: {e}"));
            assert_eq!(meta.kind, None);
        }
    }

    #[test]
    fn a_kind_round_trips_on_the_wire_in_snake_case() {
        for kind in SessionKind::ALL {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            assert_eq!(serde_json::from_str::<SessionKind>(&json).unwrap(), kind);
            assert_eq!(SessionKind::parse_lenient(kind.as_str()), Some(kind));
        }
        // Compiler-forced, not a length literal: `ALL` is `[SessionKind; 11]`,
        // so a length assert is a tautology about its own type and a
        // twelfth variant with an updated `as_str` and a forgotten `ALL`
        // entry would leave every test green while the binary read its own
        // rows back as unknown (found on review). A new variant fails this
        // match, and the arm the author then writes forces the `ALL` entry.
        fn position(k: SessionKind) -> usize {
            match k {
                SessionKind::Run => 0,
                SessionKind::Chat => 1,
                SessionKind::Tui => 2,
                SessionKind::Web => 3,
                SessionKind::Voice => 4,
                SessionKind::Task => 5,
                SessionKind::Trigger => 6,
                SessionKind::Frontdoor => 7,
                SessionKind::Mail => 8,
                SessionKind::Slack => 9,
                SessionKind::Test => 10,
            }
        }
        assert!(
            SessionKind::ALL
                .iter()
                .enumerate()
                .all(|(i, k)| position(*k) == i),
            "every variant, in `ALL`, once, in order"
        );
    }

    #[test]
    fn the_front_ends_kind_is_written_and_read_back() {
        let dir = tmpdir();
        let mut meta = meta_with_id("20260101T000000-web");
        meta.kind = Some(SessionKind::Web);
        let s = Session::create(&dir, meta).unwrap();
        assert_eq!(kind_of(&s.path), Some(SessionKind::Web));
        let none = Session::create(&dir, meta_with_id("20260101T000001-none")).unwrap();
        assert_eq!(
            kind_of(&none.path),
            None,
            "no front-end kind and no override: unknown"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `Session::create` reads the process environment, and every test in
    /// this binary shares one — a `set_var` here made `runlog`'s scans see
    /// `Test` sessions and drop them, in a different module, on the first
    /// run. So the override is exercised in a child process: this test runs
    /// the (ignored) probe below through the test binary itself with the
    /// variable set, and grades the exit status.
    #[test]
    fn the_test_override_narrows_to_test_and_never_widens_to_anything_else() {
        let exe = std::env::current_exe().unwrap();
        for (value, expect) in [("test", "test"), ("web", "run"), ("", "run")] {
            let out = std::process::Command::new(&exe)
                .args([
                    "kind_override_probe",
                    "--exact",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(SESSION_KIND_ENV, value)
                .env("MECHA_KIND_PROBE_EXPECT", expect)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{SESSION_KIND_ENV}={value:?} expecting {expect}:\n{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// The child half of the test above. Ignored so it never runs in the
    /// ordinary sweep, where it would read whatever the environment
    /// happened to hold.
    #[test]
    #[ignore]
    fn kind_override_probe() {
        let expect =
            SessionKind::parse_lenient(&std::env::var("MECHA_KIND_PROBE_EXPECT").unwrap()).unwrap();
        let dir = tmpdir();
        let mut meta = meta_with_id("20260101T000000-run");
        meta.kind = Some(SessionKind::Run);
        let s = Session::create(&dir, meta).unwrap();
        assert_eq!(
            kind_of(&s.path),
            Some(expect),
            "the override narrows and never widens"
        );
        let none = Session::create(&dir, meta_with_id("20260101T000001-none")).unwrap();
        let expect_none = (expect == SessionKind::Test).then_some(SessionKind::Test);
        assert_eq!(
            kind_of(&none.path),
            expect_none,
            "an unknown surface under the override is a test; without it, unknown"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // --- declared checks -----------------------------------------------------

    #[test]
    fn checks_are_counted_off_the_trace_by_name_and_fold_like_boredom_notices() {
        let trace = |name: &str, is_error: bool, denied: bool| crate::agent::ToolCallTrace {
            name: name.into(),
            input: serde_json::json!({}),
            is_error,
            denied,
            unknown: false,
            staged: false,
        };
        let o = crate::agent::RunOutcome {
            context_overflows: 0,
            boredom_notices: 0,
            step_escalations_attempted: 0,
            step_escalations_revised: 0,
            text: String::new(),
            stop_reason: crate::message::StopReason::EndTurn,
            usage: crate::message::Usage::default(),
            turns: 1,
            refusal: None,
            exhausted: false,
            ended_on_failed_call: false,
            tool_calls: vec![
                trace("shell", false, false),
                trace(crate::step::CHECK_TRACE, false, false),
                trace(crate::step::CHECK_TRACE, true, false),
                trace(crate::step::CHECK_TRACE, true, true),
            ],
            malformed_tool_args: 0,
            blocked_sends: 0,
            taint: crate::agent::Taint::default(),
            homeostat: None,
            stop_cause: crate::agent::StopCause::Completed,
            compactions: 0,
            usage_complete: true,
            cost_usd: None,
        };
        let stats = RunStats::from(&o);
        assert_eq!(
            (stats.checks_declared, stats.checks_passed),
            (Some(2), Some(1))
        );
        assert_eq!(
            stats.tool_calls, 4,
            "the raw count still holds every trace entry"
        );

        let mut folded = stats.clone();
        folded.merge(&stats);
        assert_eq!(
            (folded.checks_declared, folded.checks_passed),
            (Some(4), Some(2))
        );
        let old = RunStats::default();
        assert_eq!(
            old.checks_declared, None,
            "a row from before the record is unknown"
        );
        let mut mixed = old.clone();
        mixed.merge(&stats);
        assert_eq!(mixed.checks_declared, Some(2));
        let row: RunStats = serde_json::from_str(r#"{"turns":1,"usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#).unwrap();
        assert_eq!(row.checks_passed, None);
    }

    #[test]
    fn backlog_delta_sums_across_an_episodes_runs_while_the_conditions_stay_the_first_runs() {
        use crate::backlog::BacklogDelta;
        use crate::homeostat::Homeostat;
        let run = |load: f32, delta: Option<BacklogDelta>| RunStats {
            homeostat: Some(Homeostat {
                load_avg_1m: Some(load),
                backlog_delta: delta,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut folded = run(
            1.0,
            Some(BacklogDelta {
                questions: Some(1),
                ..Default::default()
            }),
        );
        folded.merge(&run(
            9.0,
            Some(BacklogDelta {
                questions: Some(-2),
                outbox: Some(-1),
                ..Default::default()
            }),
        ));
        let h = folded.homeostat.unwrap();
        assert_eq!(h.load_avg_1m, Some(1.0), "a condition: the first run's");
        assert_eq!(h.backlog_delta.unwrap().net(), Some(-2), "an act: summed");
        // A first run without the sensor takes the later run's delta rather
        // than pinning the field to absent forever.
        let mut none_first = run(1.0, None);
        none_first.merge(&run(
            2.0,
            Some(BacklogDelta {
                outbox: Some(-1),
                ..Default::default()
            }),
        ));
        assert_eq!(
            none_first.homeostat.unwrap().backlog_delta.unwrap().net(),
            Some(-1)
        );
    }
}
