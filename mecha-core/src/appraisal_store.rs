//! The text appraisal of a session, stored — grounded before it is kept,
//! carrying its run's taint, and served clean-only to every reader but the
//! owner.
//!
//! `docs/APPRAISAL-WIRING-DESIGN.md` I1, row 2a-1. An appraisal is an
//! interpretation of meaning, in text (R17): what happened relative to what
//! the run was for, why, what it means for the goal and the owner, what to
//! expect next time. A few judgments are read out of it — good or bad per
//! goal, the pointers its factual claims rest on — because arithmetic needs
//! them; an emotion label, if the appraiser uses one, is a word inside the
//! prose and never a field. There is deliberately no scalar here: no
//! valence, no score, no confidence.
//!
//! This module is the store and nothing else. Its producer is the distiller,
//! extended (R25, row 2a-2); until that lands nothing writes here but tests.
//!
//! **The write door grounds, then stamps provenance, then appends.**
//! [`AppraisalStore::record`] takes a [`Draft`] — the appraiser's unverified
//! output — and a [`SessionEvidence`] read from the transcript, never
//! constructed by a caller. Every factual [`Claim`] must cite a [`Pointer`]
//! that dereferences into what the run actually received
//! ([`crate::grounding::admit`], over the [`crate::grounding::calls`] walk):
//! a claim that does not is **dropped before storage** and counted by reason
//! on the record ([`Grounding`]), so an ungrounded claim is never stored as
//! fact and the record still says how many there were.
//!
//! **What a claim may rest on.** Two referents, both things the run
//! *received*: the result of a call the run issued (`result:<tool_use_id>`,
//! first seen wins and a stale marker never grounds — the walk's rule), and
//! an owner turn (`turn:<n>`, the user-role text of message `n` in
//! [`crate::session::Session::messages_ever`]'s order, with the harness's own
//! voice left out as the distiller's renderer leaves it out). The agent's own
//! words are not a referent: everything a model says about its own work is
//! hearsay, and a claim grounded in the assistant's "I sent it" would be
//! certified by the thing it claims. A pointer kind this build cannot read
//! is kept verbatim ([`Pointer::Unread`]) and grounds nothing.
//!
//! **Provenance is read, never supplied** (R18). The record carries the taint
//! covering the session's last message and the [`Origin`] classified from
//! it by [`crate::learning::classify_origin`] — the learning gate's own rule,
//! so no checkpoint after the last message is unknown, and unknown is
//! untrusted. A tainted session's appraisal is **stored**, not refused: it is
//! the owner's to read.
//!
//! **Two read doors, and the split is in the type** (R19). Learning, memory
//! retrieval, and credit and tenure take [`Clean`] — a wrapper whose only
//! constructor is private to this module and admits a record whose stored
//! origin is clean *and* whose stored taint is recorded and untrusted-free.
//! [`AppraisalStore::clean`] is the only way to get one. The owner's
//! surfaces read every record through [`AppraisalStore::for_owner`], as
//! plain [`TextAppraisal`]s, which no clean consumer's signature accepts.
//!
//! **It is a wire format.** Closed enums degrade to `unknown`, optional
//! fields default, an origin word this build cannot read loads as untrusted,
//! a goal of an unknown kind loads as no goal, and a torn line costs that
//! line — counted, so a reader can say the store was not fully read. An
//! unreadable file is an error, not an empty store.

use crate::agent::Taint;
use crate::goal::GoalRef;
use crate::learning::{classify_origin, Origin};
use crate::message::{Block, Message, Role};
use crate::situation::Situation;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The interpretation's ceiling. The distiller's episode is two to eight
/// sentences; an interpretation says more (what it meant, why, what next),
/// and still has to be read by a person and carried into a later prompt.
pub const INTERPRETATION_MAX_CHARS: usize = 2_000;
/// A factual claim is one statement, not a paragraph.
pub const STATEMENT_MAX_CHARS: usize = 400;
/// Below this a quote matches by accident — a name, a date. Gossip's floor
/// (`grounding::admit` takes it as a parameter and suggests none), because
/// the failure it guards is the same: a quote short enough to be anywhere.
pub const QUOTE_MIN_CHARS: usize = 12;
/// A quote is evidence, not a copy channel. Containment is an
/// anti-fabrication check, not an anti-injection one — an instruction copied
/// verbatim is a literal span too — so the ceiling is applied before it
/// (`grounding::Refusal::QuoteTooLong`'s doc).
pub const QUOTE_MAX_CHARS: usize = 300;
/// Claims considered per appraisal; the rest are dropped as `over_cap`.
pub const MAX_CLAIMS: usize = 12;
/// Goals one appraisal may judge.
pub const MAX_JUDGMENTS: usize = 8;
pub const PREDICTION_MAX_CHARS: usize = 400;
pub const HYPOTHESIS_MAX_CHARS: usize = 300;
pub const MAX_HYPOTHESES: usize = 3;
pub const LESSON_MAX_CHARS: usize = 400;
pub const MAX_LESSONS: usize = 3;

// ─── Pointers and claims ────────────────────────────────────────────────────

/// What a factual claim rests on, spelled as the referent's id in the
/// grounding packet: `result:<tool_use_id>` or `turn:<n>`.
///
/// A flat string on the wire for `GoalRef`'s reason — the model writes it,
/// and one string is harder to get wrong than an object. Reading one back
/// never fails: a kind this build cannot read is kept verbatim as
/// [`Pointer::Unread`], round-trips, and grounds nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Pointer {
    /// The result the model read for the call with this `tool_use_id`.
    Result(String),
    /// The owner's text in message `n` of `messages_ever`.
    Turn(usize),
    /// A spelling this build cannot read — a newer kind, or junk.
    Unread(String),
}

impl Pointer {
    pub fn parse(s: &str) -> Pointer {
        let s = s.trim();
        match s.split_once(':') {
            Some(("result", id)) if !id.trim().is_empty() => Pointer::Result(id.trim().to_string()),
            Some(("turn", n)) => match n.trim().parse::<usize>() {
                Ok(n) => Pointer::Turn(n),
                Err(_) => Pointer::Unread(s.to_string()),
            },
            _ => Pointer::Unread(s.to_string()),
        }
    }
}

impl std::fmt::Display for Pointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pointer::Result(id) => write!(f, "result:{id}"),
            Pointer::Turn(n) => write!(f, "turn:{n}"),
            Pointer::Unread(raw) => f.write_str(raw),
        }
    }
}

impl From<String> for Pointer {
    fn from(s: String) -> Pointer {
        Pointer::parse(&s)
    }
}

impl From<Pointer> for String {
    fn from(p: Pointer) -> String {
        p.to_string()
    }
}

/// A factual claim: what is asserted, what it rests on, and the literal
/// span of that referent it quotes. On a stored record every claim has been
/// admitted; on a [`Draft`] none has yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    #[serde(default)]
    pub statement: String,
    pub pointer: Pointer,
    #[serde(default)]
    pub quote: String,
}

/// Which way an outcome bore on a goal. Good or bad, nothing finer: a
/// magnitude is a number, and a number is what R17 took out of the
/// appraisal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Bearing {
    Good,
    Bad,
    /// A word a newer build wrote, or a field the row lacks.
    #[default]
    #[serde(other)]
    Unknown,
}

/// Good or bad for one goal the run bore on, and the claims that say so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Judgment {
    /// `None` on read when the stored goal is of a kind this build cannot
    /// read — the reference is lost, the judgment is kept.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::goal::de_lenient"
    )]
    pub goal: Option<GoalRef>,
    #[serde(default)]
    pub bearing: Bearing,
    /// Indices into the record's `claims`. On a draft they index the
    /// draft's claims; the write door renumbers them to the claims it kept
    /// and drops any that pointed at a claim it did not, so a judgment can
    /// end with no grounded support — visible as an empty list, never as a
    /// dangling index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub because: Vec<usize>,
}

/// What the appraiser wrote, before grounding. A producer builds one; only
/// [`AppraisalStore::record`] turns it into a stored record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    /// The interpretation, in prose. Empty is refused: there is nothing to
    /// keep.
    pub interpretation: String,
    pub judgments: Vec<Judgment>,
    pub claims: Vec<Claim>,
    /// What to expect next time in this situation — scored when next time
    /// comes (X5, row 2b).
    pub prediction: Option<String>,
    /// What the owner's reactions suggest they want (I4) — hypotheses, never
    /// a goal the run serves.
    pub goal_hypotheses: Vec<String>,
    /// What to do differently, for the learner (row 2e) — clean records
    /// only reach it.
    pub lessons: Vec<String>,
}

// ─── Provenance and the grounding packet ────────────────────────────────────

/// What the transcript says about a session, read once, for the write door:
/// its provenance, its anchor and situation, and the packet a claim must
/// dereference into. Fields are private — a caller cannot assert clean.
#[derive(Debug, Clone)]
pub struct SessionEvidence {
    session_id: String,
    taint: Option<Taint>,
    origin: Origin,
    anchor: Option<GoalRef>,
    situation: Option<Situation>,
    packet: Vec<crate::grounding::Evidence>,
}

impl SessionEvidence {
    /// Read a transcript file: the one-pass [`crate::session::Session::read`]
    /// for provenance, anchor and run config, and
    /// [`crate::session::Session::messages_ever`] for the referents —
    /// what compaction later evicted was still received.
    pub fn read(path: &Path) -> Result<SessionEvidence> {
        let transcript = crate::session::Session::read(path)?;
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let ever = crate::session::Session::messages_ever(&text);
        Ok(SessionEvidence::of(&transcript, &ever))
    }

    /// The same, from a transcript already read and its `messages_ever`.
    pub fn of(transcript: &crate::session::Transcript, ever: &[Message]) -> SessionEvidence {
        // Taint only grows, so the checkpoint covering the last message
        // covers the session; none after it is unknown, which is untrusted.
        let messages = transcript.convo.messages.len();
        let taint = (messages > 0)
            .then(|| transcript.taint_timeline.covering(messages - 1))
            .flatten();
        // The run's situation, from the last run record's matched keys —
        // `None` when no config was recorded: unknown, never the
        // empty-keyed scope, which is standing and would match every run.
        let situation = transcript
            .configs
            .last()
            .map(|c| Situation::of_run(&c.tools, c.rules_workspace.as_deref()).on(c.rules_surface));
        SessionEvidence {
            session_id: transcript.meta.id.clone(),
            taint,
            origin: classify_origin(taint),
            anchor: transcript.convo.goal_anchor.clone(),
            situation,
            packet: packet(ever),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    pub fn taint(&self) -> Option<Taint> {
        self.taint
    }

    /// The referent ids a claim may cite, in packet order — what a producer
    /// shows the appraiser beside the transcript.
    pub fn referent_ids(&self) -> impl Iterator<Item = &str> {
        self.packet.iter().map(|e| e.id.as_str())
    }
}

/// The referents a claim may cite: every surviving call result, and every
/// owner turn. See the module doc for why the agent's words are not here.
fn packet(ever: &[Message]) -> Vec<crate::grounding::Evidence> {
    let mut out: Vec<crate::grounding::Evidence> = crate::grounding::calls(ever)
        .into_iter()
        .filter_map(|call| {
            call.result.map(|text| crate::grounding::Evidence {
                id: Pointer::Result(call.id.to_string()).to_string(),
                source: call.name.to_string(),
                text: text.to_string(),
            })
        })
        .collect();
    for (index, message) in ever.iter().enumerate() {
        if message.role != Role::User || message.harness {
            continue;
        }
        let owner: Vec<&str> = message
            .content
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .filter(|t| {
                !t.trim().is_empty()
                    && !crate::agent::is_harness_voice(t)
                    && !t
                        .trim_start()
                        .starts_with(crate::date_context::REFERENCE_STEM)
            })
            .collect();
        if owner.is_empty() {
            continue;
        }
        out.push(crate::grounding::Evidence {
            id: Pointer::Turn(index).to_string(),
            source: "owner".into(),
            text: owner.join("\n"),
        });
    }
    out
}

// ─── The record ─────────────────────────────────────────────────────────────

/// How the write door's grounding went: every claim the draft offered was
/// either kept (it is in `claims`) or dropped and counted here by reason.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grounding {
    /// Claims the draft offered.
    #[serde(default)]
    pub offered: usize,
    /// Claims dropped before storage.
    #[serde(default)]
    pub dropped: usize,
    /// Dropped claims by reason: a [`crate::grounding::Refusal`] by its
    /// wire name, or `unknown_pointer`, `statement_too_long`, `over_cap`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dropped_by: BTreeMap<String, usize>,
}

/// One session's text appraisal, as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextAppraisal {
    pub id: String,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub session_id: String,
    /// Classified from `taint` at write. An origin word this build cannot
    /// read, or none at all, loads as untrusted.
    #[serde(default = "untrusted", deserialize_with = "de_origin")]
    pub origin: Origin,
    /// The taint covering the session's last message; `None` is unknown.
    #[serde(default)]
    pub taint: Option<Taint>,
    /// The session's goal anchor — the last one, as `comparison.rs` takes
    /// it: the transcript keeps no anchor positions. Harness-minted, so the
    /// id is kept (retrieval by goal needs it).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::goal::de_lenient"
    )]
    pub anchor: Option<GoalRef>,
    /// The run's situation from its last run record; `None` is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub situation: Option<Situation>,
    /// The model that wrote the draft.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub interpretation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub judgments: Vec<Judgment>,
    /// Grounded claims only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub grounding: Grounding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goal_hypotheses: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lessons: Vec<String>,
    /// A bound cut something: the interpretation, the prediction, a
    /// hypothesis or lesson, or the number of either. Claims are never cut —
    /// one over a bound is dropped and counted in `grounding`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clipped: bool,
}

fn untrusted() -> Origin {
    Origin::Untrusted
}

/// An origin word from the file, failing closed: `clean` and `derived` are
/// read as themselves, anything else — a newer word, junk, a non-string —
/// as untrusted, never as a failed row.
fn de_origin<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Origin, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v.as_str() {
        Some("clean") => Origin::Clean,
        Some("derived") => Origin::Derived,
        _ => Origin::Untrusted,
    })
}

impl TextAppraisal {
    /// The one predicate the clean door uses: the stored origin is clean
    /// **and** the stored taint is recorded and carries no untrusted
    /// content. Both are written from the same reading, so they agree on
    /// every row this module wrote; requiring both means a row where they
    /// disagree — a hand edit, a torn field — is not served as clean.
    pub fn is_clean(&self) -> bool {
        self.origin == Origin::Clean && matches!(self.taint, Some(t) if !t.untrusted)
    }

    /// Ground `draft` against `evidence` and seal it. `None` when the draft
    /// has no interpretation to keep.
    fn seal(evidence: &SessionEvidence, draft: Draft, model: &str) -> Option<TextAppraisal> {
        let mut clipped = false;
        let interpretation = bound(
            &draft.interpretation,
            INTERPRETATION_MAX_CHARS,
            &mut clipped,
        )?;

        let mut grounding = Grounding {
            offered: draft.claims.len(),
            ..Grounding::default()
        };
        let mut drop = |reason: String| {
            grounding.dropped += 1;
            *grounding.dropped_by.entry(reason).or_default() += 1;
        };
        // Old index → new index, for the claims kept.
        let mut kept_at: BTreeMap<usize, usize> = BTreeMap::new();
        let mut claims: Vec<Claim> = Vec::new();
        for (i, claim) in draft.claims.into_iter().enumerate() {
            if i >= MAX_CLAIMS {
                drop("over_cap".into());
                continue;
            }
            match ground(&claim, &evidence.packet) {
                Ok(()) => {
                    kept_at.insert(i, claims.len());
                    claims.push(Claim {
                        statement: claim.statement.trim().to_string(),
                        pointer: claim.pointer,
                        quote: claim.quote.trim().trim_matches('"').to_string(),
                    });
                }
                Err(reason) => drop(reason),
            }
        }

        if draft.judgments.len() > MAX_JUDGMENTS {
            clipped = true;
        }
        let judgments = draft
            .judgments
            .into_iter()
            .take(MAX_JUDGMENTS)
            .map(|j| Judgment {
                because: j
                    .because
                    .iter()
                    .filter_map(|i| kept_at.get(i).copied())
                    .collect(),
                ..j
            })
            .collect();

        let prediction = draft
            .prediction
            .as_deref()
            .and_then(|p| bound(p, PREDICTION_MAX_CHARS, &mut clipped));
        let goal_hypotheses = bound_list(
            &draft.goal_hypotheses,
            MAX_HYPOTHESES,
            HYPOTHESIS_MAX_CHARS,
            &mut clipped,
        );
        let lessons = bound_list(&draft.lessons, MAX_LESSONS, LESSON_MAX_CHARS, &mut clipped);

        Some(TextAppraisal {
            id: format!("apr-{}", uuid::Uuid::new_v4()),
            at: Utc::now(),
            session_id: evidence.session_id.clone(),
            origin: evidence.origin,
            taint: evidence.taint,
            anchor: evidence.anchor.clone(),
            situation: evidence.situation.clone(),
            model: model.to_string(),
            interpretation,
            judgments,
            claims,
            grounding,
            prediction,
            goal_hypotheses,
            lessons,
            clipped,
        })
    }
}

/// Dereference one claim, or say why not.
fn ground(claim: &Claim, packet: &[crate::grounding::Evidence]) -> Result<(), String> {
    let name = |r: crate::grounding::Refusal| crate::appraisal::enum_name(&r);
    if matches!(claim.pointer, Pointer::Unread(_)) {
        return Err("unknown_pointer".into());
    }
    if claim.statement.trim().chars().count() > STATEMENT_MAX_CHARS {
        return Err("statement_too_long".into());
    }
    // The ceiling before the check, as the front door and triage apply it.
    if claim.quote.trim().trim_matches('"').chars().count() > QUOTE_MAX_CHARS {
        return Err(name(crate::grounding::Refusal::QuoteTooLong));
    }
    let id = claim.pointer.to_string();
    crate::grounding::admit(
        &crate::grounding::Claim {
            statement: &claim.statement,
            id: &id,
            quote: &claim.quote,
        },
        packet,
        QUOTE_MIN_CHARS,
    )
    .map(|_| ())
    .map_err(name)
}

/// Trimmed, empty as `None`, cut at `max` characters (flagging the cut).
fn bound(text: &str, max: usize, clipped: &mut bool) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() > max {
        *clipped = true;
        return Some(text.chars().take(max).collect());
    }
    Some(text.to_string())
}

fn bound_list(items: &[String], count: usize, max: usize, clipped: &mut bool) -> Vec<String> {
    let kept: Vec<String> = items
        .iter()
        .filter_map(|s| bound(s, max, clipped))
        .collect();
    if kept.len() > count {
        *clipped = true;
    }
    kept.into_iter().take(count).collect()
}

// ─── The clean door ─────────────────────────────────────────────────────────

/// A text appraisal of a clean run — the only shape learning, memory
/// retrieval, credit and tenure may take (R19).
///
/// The field is private and so is the constructor: the one way to hold a
/// `Clean` is [`AppraisalStore::clean`], which admits a record only through
/// [`TextAppraisal::is_clean`]. A consumer whose signature takes `Clean`
/// cannot be handed a tainted appraisal, whatever its caller holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Clean(TextAppraisal);

impl Clean {
    fn admit(record: TextAppraisal) -> Option<Clean> {
        record.is_clean().then_some(Clean(record))
    }

    pub fn get(&self) -> &TextAppraisal {
        &self.0
    }
}

impl std::ops::Deref for Clean {
    type Target = TextAppraisal;
    fn deref(&self) -> &TextAppraisal {
        &self.0
    }
}

/// What the clean door returned, and what it held back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleanRead {
    pub appraisals: Vec<Clean>,
    /// Records of runs that were not clean, or whose provenance is unknown:
    /// stored, the owner's to read, and never here.
    pub withheld: usize,
    /// Lines that could not be read at all.
    pub skipped: usize,
}

// ─── The store ──────────────────────────────────────────────────────────────

/// What [`AppraisalStore::record`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// Appended. `clean` says which door will serve it; `grounding` what
    /// the write door dropped.
    Written {
        id: String,
        clean: bool,
        grounding: Grounding,
    },
    /// The draft carried no interpretation. Nothing was written.
    Empty,
}

/// The append-only text-appraisal store, `~/.mecha/appraisals/appraisals.jsonl`.
pub struct AppraisalStore {
    root: PathBuf,
}

struct StoreLock {
    _file: std::fs::File,
}

impl AppraisalStore {
    /// `~/.mecha/appraisals`, under [`crate::work::mecha_home`] (which
    /// honours `MECHA_HOME`, so a trial home keeps its own).
    pub fn default_root() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("appraisals"))
    }

    /// Open, creating the directory — a producer opens before it pays for
    /// the model call whose output it could not otherwise keep.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        Ok(AppraisalStore { root })
    }

    pub fn open_default() -> Result<Self> {
        Self::open(Self::default_root()?)
    }

    /// Open at the default location only if it already exists — a read path
    /// must not create the store it is about to report on.
    pub fn open_existing_default() -> Option<Self> {
        let root = Self::default_root().ok()?;
        root.is_dir().then_some(AppraisalStore { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn ledger(&self) -> PathBuf {
        self.root.join("appraisals.jsonl")
    }

    fn lock(&self) -> Result<StoreLock> {
        use std::os::unix::io::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))?;
        // SAFETY: flock on an fd we own, held open by the returned guard.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).context("locking the appraisal store");
        }
        Ok(StoreLock { _file: file })
    }

    /// The one write door: ground `draft` against `evidence`, stamp the
    /// session's recorded provenance, and append under the lock, synced. A
    /// tainted session's appraisal is written — it is the owner's — and the
    /// clean door will never serve it. An I/O failure is an `Err`, never a
    /// quiet no-op: a caller that paid a model call must hear it was lost.
    pub fn record(
        &self,
        evidence: &SessionEvidence,
        draft: Draft,
        model: &str,
    ) -> Result<Recorded> {
        let Some(record) = TextAppraisal::seal(evidence, draft, model) else {
            return Ok(Recorded::Empty);
        };
        use std::io::Write;
        let _lock = self.lock()?;
        let path = self.ledger();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        let mut line = serde_json::to_string(&record)?;
        line.push('\n');
        file.write_all(line.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        file.sync_data()
            .with_context(|| format!("syncing {}", path.display()))?;
        Ok(Recorded::Written {
            clean: record.is_clean(),
            id: record.id,
            grounding: record.grounding,
        })
    }

    /// Every record, oldest first, and how many lines were skipped — **for
    /// the owner's surfaces only**. A missing file is an empty store; a file
    /// that cannot be read is an `Err`.
    pub fn for_owner(&self) -> Result<(Vec<TextAppraisal>, usize)> {
        let path = self.ledger();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut out = Vec::new();
        let mut skipped = 0usize;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str(line) {
                Ok(r) => out.push(r),
                Err(e) => {
                    skipped += 1;
                    tracing::warn!("skipping unreadable appraisal row: {e}");
                }
            }
        }
        Ok((out, skipped))
    }

    /// The clean appraisals, oldest first — the door for learning, memory
    /// retrieval, credit and tenure. Everything else is counted as withheld.
    pub fn clean(&self) -> Result<CleanRead> {
        let (rows, skipped) = self.for_owner()?;
        let mut read = CleanRead {
            skipped,
            ..CleanRead::default()
        };
        for row in rows {
            match Clean::admit(row) {
                Some(clean) => read.appraisals.push(clean),
                None => read.withheld += 1,
            }
        }
        Ok(read)
    }
}

/// The store, counted: what a readout prints. Counts only — no prose.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    pub records: usize,
    /// Distinct sessions appraised.
    pub sessions: usize,
    /// Records the clean door serves.
    pub clean: usize,
    /// Records the owner alone reads: tainted, or of unknown provenance.
    pub not_clean: usize,
    /// Grounded claims kept, across records.
    pub claims_kept: usize,
    /// Claims dropped before storage, across records, and by reason.
    pub claims_dropped: usize,
    pub dropped_by: BTreeMap<String, usize>,
    /// Records a bound cut.
    pub clipped: usize,
}

impl Summary {
    pub fn of(rows: &[TextAppraisal]) -> Summary {
        let mut s = Summary {
            records: rows.len(),
            ..Summary::default()
        };
        let mut sessions = std::collections::BTreeSet::new();
        for r in rows {
            sessions.insert(r.session_id.as_str());
            if r.is_clean() {
                s.clean += 1;
            } else {
                s.not_clean += 1;
            }
            s.claims_kept += r.claims.len();
            s.claims_dropped += r.grounding.dropped;
            for (reason, n) in &r.grounding.dropped_by {
                *s.dropped_by.entry(reason.clone()).or_default() += n;
            }
            if r.clipped {
                s.clipped += 1;
            }
        }
        s.sessions = sessions.len();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compact::SUPERSEDED_MARKER;
    use crate::session::{Record, RunConfig, Session, SessionKind, SessionMeta};
    use serde_json::json;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-appraisals-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    const INBOX: &str = "From Dana Rowe: the budget review moved to Thursday at 10.";

    /// A session as a front-end records one: the owner's ask, a call and its
    /// result, a steer, the agent's answer, the run record and — when given
    /// — a taint checkpoint after the last message.
    fn session(root: &Path, taint: Option<Taint>) -> PathBuf {
        let session = Session::create(
            root,
            SessionMeta {
                id: Session::new_id(),
                created_at: Utc::now(),
                provider: "scripted".into(),
                model: "scripted".into(),
                workspace: root.to_path_buf(),
                title: None,
                kind: Some(SessionKind::Task),
            },
        )
        .unwrap();
        session
            .append(&Record::Config(RunConfig {
                tools: vec!["mail_search".into()],
                rules_workspace: Some(PathBuf::from("/project")),
                rules_surface: Some(SessionKind::Task),
                ..Default::default()
            }))
            .unwrap();
        session
            .append(&Record::GoalAnchor {
                goal: Some(GoalRef::Task("t-budget".into())),
            })
            .unwrap();
        for m in [
            Message::user("find when the budget review is"),
            Message::assistant(vec![Block::ToolUse {
                id: "t1".into(),
                name: "mail_search".into(),
                input: json!({"query": "budget review"}),
            }]),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: INBOX.into(),
                is_error: false,
            }]),
            Message::user("and tell Idris it is Thursday"),
            Message::assistant(vec![Block::text(
                "I told Idris the review is on Friday at noon.",
            )]),
        ] {
            session.append(&Record::Message(m)).unwrap();
        }
        if let Some(t) = taint {
            session.append(&Record::Taint(t)).unwrap();
        }
        session.path
    }

    fn clean_taint() -> Option<Taint> {
        Some(Taint {
            private: true,
            untrusted: false,
        })
    }

    fn tainted() -> Option<Taint> {
        Some(Taint {
            private: true,
            untrusted: true,
        })
    }

    fn claim(statement: &str, pointer: &str, quote: &str) -> Claim {
        Claim {
            statement: statement.into(),
            pointer: Pointer::parse(pointer),
            quote: quote.into(),
        }
    }

    /// One claim that dereferences into a call result and one into an owner
    /// turn, a judgment resting on both.
    fn draft() -> Draft {
        Draft {
            interpretation: "The run found the review date in the owner's mail and was asked \
                             to pass it on; it matters because the task is the budget review."
                .into(),
            judgments: vec![Judgment {
                goal: Some(GoalRef::Task("t-budget".into())),
                bearing: Bearing::Good,
                because: vec![0, 1],
            }],
            claims: vec![
                claim(
                    "The review moved to Thursday",
                    "result:t1",
                    "the budget review moved to Thursday",
                ),
                claim(
                    "The owner asked to tell Idris",
                    "turn:3",
                    "tell Idris it is Thursday",
                ),
            ],
            prediction: Some("Next time the owner will want the date confirmed.".into()),
            goal_hypotheses: vec!["The owner wants colleagues kept informed.".into()],
            lessons: vec!["Quote the date from the mail when passing it on.".into()],
        }
    }

    /// The acceptance, at the store: an appraisal of a clean session reads
    /// back through a fresh handle, via the clean door, field for field —
    /// with its provenance, anchor and situation read off the transcript.
    #[test]
    fn a_clean_appraisal_reads_back_through_a_fresh_handle_via_the_clean_door() {
        let root = temp_root("clean");
        let path = session(&root.join("sessions"), clean_taint());
        let evidence = SessionEvidence::read(&path).unwrap();
        assert_eq!(evidence.origin(), Origin::Clean);
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        let Recorded::Written {
            id,
            clean,
            grounding,
        } = store.record(&evidence, draft(), "local-model").unwrap()
        else {
            panic!("written");
        };
        assert!(clean);
        assert_eq!((grounding.offered, grounding.dropped), (2, 0));

        let again = AppraisalStore::open(root.join("appraisals")).unwrap();
        let read = again.clean().unwrap();
        assert_eq!((read.withheld, read.skipped), (0, 0));
        assert_eq!(read.appraisals.len(), 1);
        let got = &read.appraisals[0];
        assert_eq!(got.id, id);
        assert_eq!(got.session_id, evidence.session_id());
        assert_eq!(got.origin, Origin::Clean);
        assert_eq!(got.taint, clean_taint());
        assert_eq!(got.anchor, Some(GoalRef::Task("t-budget".into())));
        let situation = got.situation.as_ref().expect("read off the run record");
        assert_eq!(situation.surface, Some(SessionKind::Task));
        assert_eq!(situation.workspace, Some(PathBuf::from("/project")));
        assert_eq!(got.claims.len(), 2);
        assert_eq!(got.claims[0].pointer, Pointer::Result("t1".into()));
        assert_eq!(got.claims[1].pointer, Pointer::Turn(3));
        assert_eq!(got.judgments[0].bearing, Bearing::Good);
        assert_eq!(got.judgments[0].because, vec![0, 1]);
        assert_eq!(got.model, "local-model");
        assert!(got.prediction.is_some() && !got.clipped);
        assert_eq!(
            again.for_owner().unwrap().0,
            vec![got.get().clone()],
            "the owner's door sees the same record"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Tainted, and unknown (no checkpoint after the last message): each is
    /// stored — the owner reads it — and neither is ever served clean.
    #[test]
    fn a_tainted_or_unknown_appraisal_is_stored_but_never_served_clean() {
        let root = temp_root("tainted");
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        for taint in [tainted(), None] {
            let path = session(&root.join("sessions"), taint);
            let evidence = SessionEvidence::read(&path).unwrap();
            assert_eq!(evidence.origin(), Origin::Untrusted, "{taint:?}");
            assert!(matches!(
                store.record(&evidence, draft(), "m").unwrap(),
                Recorded::Written { clean: false, .. }
            ));
        }
        let (owner, _) = store.for_owner().unwrap();
        assert_eq!(owner.len(), 2, "both are stored");
        assert_eq!(owner[1].taint, None, "unknown is recorded as unknown");
        let read = AppraisalStore::open(root.join("appraisals"))
            .unwrap()
            .clean()
            .unwrap();
        assert!(read.appraisals.is_empty());
        assert_eq!(read.withheld, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The write door drops every claim that does not dereference, before
    /// anything is written, and says why: a call the run never issued, a
    /// quote not in its referent, the agent's own words (hearsay, not a
    /// referent), a result compaction staled, a pointer kind this build
    /// cannot read, a quote under the floor and one over the ceiling, and
    /// claims past the cap. The judgment keeps only the support that stayed.
    #[test]
    fn a_claim_that_does_not_dereference_is_dropped_before_storage_and_counted() {
        let root = temp_root("grounding");
        let dir = root.join("sessions");
        let path = session(&dir, clean_taint());
        // A second result the run received, later staled by compaction.
        {
            let s = Session {
                meta: Session::read(&path).unwrap().meta,
                path: path.clone(),
            };
            s.append(&Record::Message(Message::assistant(vec![Block::ToolUse {
                id: "t2".into(),
                name: "mail_search".into(),
                input: json!({}),
            }])))
            .unwrap();
            s.append(&Record::Message(Message::tool_results(vec![
                Block::ToolResult {
                    tool_use_id: "t2".into(),
                    content: format!("{SUPERSEDED_MARKER} a later call covered it.]"),
                    is_error: false,
                },
            ])))
            .unwrap();
            s.append(&Record::Taint(clean_taint().unwrap())).unwrap();
        }
        let evidence = SessionEvidence::read(&path).unwrap();
        let mut d = draft();
        d.judgments[0].because = vec![0, 2, 1, 5];
        d.claims = vec![
            claim("Kept", "result:t1", "the budget review moved to Thursday"),
            claim("No such call", "result:t9", "the budget review moved"),
            claim("Owner said", "turn:3", "tell Idris it is Thursday"),
            claim(
                "Misquoted",
                "result:t1",
                "the budget review moved to Friday",
            ),
            claim("Hearsay", "turn:4", "I told Idris the review"),
            claim("Stale", "result:t2", "a later call covered it"),
            claim("Future kind", "draft:d-12", "the budget review moved"),
            claim("Too short", "result:t1", "Thursday"),
            claim("Too long", "result:t1", &"x".repeat(QUOTE_MAX_CHARS + 1)),
        ];
        for _ in 0..(MAX_CLAIMS - d.claims.len() + 2) {
            d.claims
                .push(claim("Filler", "result:t1", "budget review moved"));
        }
        let offered = d.claims.len();
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        let Recorded::Written { grounding, .. } = store.record(&evidence, d, "m").unwrap() else {
            panic!("written");
        };
        let fillers_kept = MAX_CLAIMS - 9;
        assert_eq!(grounding.offered, offered);
        assert_eq!(grounding.dropped, offered - 2 - fillers_kept);
        let by = |k: &str| grounding.dropped_by.get(k).copied().unwrap_or(0);
        assert_eq!(
            by("no_such_referent"),
            3,
            "t9, the agent's turn, the stale result"
        );
        assert_eq!(by("quote_not_in_referent"), 1);
        assert_eq!(by("unknown_pointer"), 1);
        assert_eq!(by("quote_too_short"), 1);
        assert_eq!(by("quote_too_long"), 1);
        assert_eq!(by("over_cap"), 2);

        let rows = store.for_owner().unwrap().0;
        let got = &rows[0];
        assert_eq!(
            got.grounding, grounding,
            "the record says how many were dropped"
        );
        let statements: Vec<&str> = got.claims.iter().map(|c| c.statement.as_str()).collect();
        assert_eq!(statements[..2], ["Kept", "Owner said"]);
        assert_eq!(
            got.judgments[0].because,
            vec![0, 1],
            "renumbered; dropped support gone"
        );
        let raw = std::fs::read_to_string(store.ledger()).unwrap();
        for gone in [
            "No such call",
            "Misquoted",
            "Hearsay",
            "Stale",
            "Future kind",
        ] {
            assert!(!raw.contains(gone), "{gone} was stored: {raw}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The harness's own voice in a user-role message is not the owner, and
    /// the agent's text is not a referent at all.
    #[test]
    fn only_call_results_and_the_owners_own_words_are_referents() {
        let mut nudged = Message::user(crate::agent::EMPTY_TURN_NUDGE);
        nudged.harness = false;
        let mut harness = Message::user("a delivered note from the harness");
        harness.harness = true;
        let ever = vec![
            Message::user("draft the note to Priya"),
            nudged,
            harness,
            Message::assistant(vec![Block::text("I drafted it.")]),
        ];
        let ids: Vec<String> = packet(&ever).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec!["turn:0".to_string()]);
    }

    /// A newer build's words, fields this build lacks, and a torn line: each
    /// row loads, each unknown degrades toward "not clean" or "no value",
    /// and the torn line costs only itself.
    #[test]
    fn old_and_unknown_variants_load_leniently_and_fail_closed() {
        let root = temp_root("lenient");
        let store = AppraisalStore::open(&root).unwrap();
        let raw = r#"{"id":"apr-future","at":"2026-09-25T00:00:00Z","session_id":"s1","origin":"clean-verified","taint":{"private":false,"untrusted":false},"anchor":"dream:x","interpretation":"i","judgments":[{"goal":"quest:y","bearing":"ecstatic","because":[0]}],"claims":[{"statement":"s","pointer":"draft:d-9","quote":"q"}],"grounding":{"offered":1,"dropped":0},"mood":"a field from later"}
{"id":"apr-sparse","at":"2026-09-25T00:00:00Z"}
{"id":"apr-lies","at":"2026-09-25T00:00:00Z","origin":"clean","taint":{"private":false,"untrusted":true},"interpretation":"hand edited"}
{"id":"apr-no-taint","at":"2026-09-25T00:00:00Z","origin":"clean","interpretation":"taint unknown"}
{"id":"apr-torn","at":"2026-09-25T00:00:00Z","origin":"cle
"#;
        std::fs::write(store.ledger(), raw).unwrap();
        let (rows, skipped) = store.for_owner().unwrap();
        assert_eq!(skipped, 1, "the torn line is counted");
        assert_eq!(rows.len(), 4, "and costs only itself");
        let future = &rows[0];
        assert_eq!(
            future.origin,
            Origin::Untrusted,
            "an unread origin word is not clean"
        );
        assert_eq!(future.anchor, None, "an unknown goal kind is no goal");
        assert_eq!(future.judgments[0].goal, None);
        assert_eq!(future.judgments[0].bearing, Bearing::Unknown);
        assert_eq!(
            future.claims[0].pointer,
            Pointer::Unread("draft:d-9".into())
        );
        let sparse = &rows[1];
        assert_eq!(
            (sparse.origin, sparse.taint, sparse.interpretation.as_str()),
            (Origin::Untrusted, None, ""),
            "a row with no provenance is unknown, and unknown is untrusted"
        );
        assert_eq!(sparse.grounding, Grounding::default());
        // A pointer this build cannot read round-trips verbatim.
        let back: Claim =
            serde_json::from_str(&serde_json::to_string(&future.claims[0]).unwrap()).unwrap();
        assert_eq!(back.pointer.to_string(), "draft:d-9");
        let read = store.clean().unwrap();
        assert!(
            read.appraisals.is_empty(),
            "no row here is clean on both counts: {:?}",
            read.appraisals
        );
        assert_eq!((read.withheld, read.skipped), (4, 1));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An empty interpretation keeps nothing; the bounds cut and say so.
    #[test]
    fn an_empty_interpretation_is_refused_and_every_bound_is_flagged() {
        let root = temp_root("bounds");
        let path = session(&root.join("sessions"), clean_taint());
        let evidence = SessionEvidence::read(&path).unwrap();
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        let empty = Draft {
            interpretation: "   ".into(),
            ..draft()
        };
        assert_eq!(
            store.record(&evidence, empty, "m").unwrap(),
            Recorded::Empty
        );
        assert!(!store.ledger().exists(), "nothing was written");

        let long = Draft {
            interpretation: "é".repeat(INTERPRETATION_MAX_CHARS + 5),
            goal_hypotheses: vec!["h".into(); MAX_HYPOTHESES + 1],
            ..draft()
        };
        store.record(&evidence, long, "m").unwrap();
        let got = &store.for_owner().unwrap().0[0];
        assert!(got.clipped);
        assert_eq!(got.interpretation.chars().count(), INTERPRETATION_MAX_CHARS);
        assert_eq!(got.goal_hypotheses.len(), MAX_HYPOTHESES);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_summary_counts_the_doors_and_the_drops() {
        let root = temp_root("summary");
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        for taint in [clean_taint(), tainted()] {
            let path = session(&root.join("sessions"), taint);
            let evidence = SessionEvidence::read(&path).unwrap();
            let mut d = draft();
            d.claims
                .push(claim("Nope", "result:t9", "the budget review moved"));
            store.record(&evidence, d, "m").unwrap();
        }
        let s = Summary::of(&store.for_owner().unwrap().0);
        assert_eq!((s.records, s.sessions, s.clean, s.not_clean), (2, 2, 1, 1));
        assert_eq!((s.claims_kept, s.claims_dropped), (4, 2));
        assert_eq!(s.dropped_by.get("no_such_referent"), Some(&2));
        assert_eq!(Summary::of(&[]), Summary::default());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_store_is_a_finding_not_an_empty_one() {
        let root = temp_root("unreadable");
        let store = AppraisalStore::open(&root).unwrap();
        std::fs::create_dir_all(store.ledger()).unwrap();
        assert!(store.for_owner().is_err());
        assert!(store.clean().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pointer_parses_leniently_and_round_trips() {
        assert_eq!(Pointer::parse(" result:t1 "), Pointer::Result("t1".into()));
        assert_eq!(Pointer::parse("turn:7"), Pointer::Turn(7));
        assert_eq!(Pointer::parse("turn:x"), Pointer::Unread("turn:x".into()));
        assert_eq!(Pointer::parse("result:"), Pointer::Unread("result:".into()));
        assert_eq!(Pointer::parse("t1"), Pointer::Unread("t1".into()));
        for s in ["result:a:b", "turn:0", "closure:c-1"] {
            assert_eq!(Pointer::parse(s).to_string(), s);
        }
    }
}
