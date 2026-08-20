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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
        self.turns += o.turns;
        self.usage.add(&o.usage);
        self.cost_usd = match (self.cost_usd, o.cost_usd) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.usage_complete &= o.usage_complete;
        self.stop_cause = Some(o.stop_cause);
        self.exhausted = o.exhausted;
        self.ended_on_failed_call = o.ended_on_failed_call;
        self.tool_calls += o.tool_calls.len() as u32;
        // `denied` is excluded, and the exclusion has to be written out: a
        // denied trace carries `is_error: true` too, so filtering on
        // `is_error` alone counts every refusal as an environment failure and
        // averages "the harness working" into the rate the candidate gate and
        // doctor both threshold on.
        self.tool_errors += o
            .tool_calls
            .iter()
            .filter(|c| c.unknown || (c.is_error && !c.denied))
            .count() as u32;
        self.tool_denied += o.tool_calls.iter().filter(|c| c.denied).count() as u32;
        self.tool_staged += o.tool_calls.iter().filter(|c| c.staged).count() as u32;
        self.malformed_tool_args += o.malformed_tool_args;
        self.blocked_sends += o.blocked_sends;
        self.compactions += o.compactions;
        self.taint.merge(o.taint);
    }
}

impl From<&crate::agent::RunOutcome> for RunStats {
    fn from(o: &crate::agent::RunOutcome) -> Self {
        // `usage_complete` starts true and is ANDed down, so the default's
        // `false` would make every single-run row a lower bound.
        let mut stats = RunStats {
            usage_complete: true,
            ..RunStats::default()
        };
        stats.absorb(o);
        stats
    }
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
}

pub struct Session {
    pub meta: SessionMeta,
    pub path: PathBuf,
}

impl Session {
    /// Where transcripts live: `~/.mecha/sessions`, or `$MECHA_SESSION_DIR`.
    pub fn default_dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("MECHA_SESSION_DIR") {
            return Ok(PathBuf::from(dir));
        }
        Ok(crate::work::mecha_home()?.join("sessions"))
    }

    pub fn create(dir: &Path, meta: SessionMeta) -> Result<Self> {
        crate::create_private_dir(dir)
            .with_context(|| format!("creating session directory {}", dir.display()))?;
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
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

        let mut meta = None;
        let mut messages = Vec::new();
        let mut taint = Taint::default();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Record>(line) {
                Ok(Record::Meta(m)) => meta = Some(m),
                Ok(Record::Message(m)) => messages.push(m),
                // The conversation state as of the rewrite, wholesale. Taint
                // is deliberately not touched: summarising away the text of a
                // hostile page does not un-read it.
                Ok(Record::Rewrite { messages: m }) => messages = m,
                // Merged rather than replaced: taint only ever grows, and a
                // transcript written by an older build has none at all.
                Ok(Record::Taint(t)) => taint.merge(t),
                Ok(Record::Summary { .. }) | Ok(Record::Config(_)) | Ok(Record::Outcome(_)) => {}
                Err(e) => tracing::warn!(error = %e, "skipping malformed transcript line"),
            }
        }

        let meta = meta.with_context(|| format!("{} has no session header", path.display()))?;
        Ok((meta, Conversation::resumed(messages, taint)))
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
            match serde_json::from_str::<Record>(line) {
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
    pub fn list(dir: &Path) -> Result<Vec<(SessionMeta, PathBuf)>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // A transcript with no header is unusable; skip it quietly.
            if let Some(meta) = Session::peek_meta(&path) {
                out.push((meta, path));
            }
        }
        out.sort_by_key(|(meta, _)| std::cmp::Reverse(meta.created_at));
        Ok(out)
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
mod tests {
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

    #[test]
    fn one_lower_bound_turn_makes_the_whole_episode_a_lower_bound() {
        use crate::agent::{RunOutcome, StopCause};
        use crate::message::StopReason;

        let mut incomplete = RunOutcome {
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
}
