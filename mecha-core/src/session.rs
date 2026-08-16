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
    /// before the run starts. When the run only appended, the new tail is
    /// appended here too. When it rewrote what was already recorded —
    /// compaction, eviction, thinning, all of which edit earlier messages in
    /// place — a [`Record::Rewrite`] carries the whole current list instead,
    /// because slicing a rewritten transcript records a lie: the old head
    /// stays in the file, the rebuilt one (summary included) never lands.
    ///
    /// Comparison, not a flag from the loop: any mutation the loop grows
    /// later is caught by construction, and the clone this costs is one more
    /// beside the one the loop already pays per request.
    pub fn record_run(&self, before: &[Message], after: &[Message]) -> Result<()> {
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
                Ok(Record::Summary { .. }) | Ok(Record::Config(_)) => {}
                Err(e) => tracing::warn!(error = %e, "skipping malformed transcript line"),
            }
        }

        let meta = meta.with_context(|| format!("{} has no session header", path.display()))?;
        Ok((meta, Conversation::resumed(messages, taint)))
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

        let mut after = before.clone();
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
        let after = vec![head, Message::assistant(vec![Block::text("done")])];
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
