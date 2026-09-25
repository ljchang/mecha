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
//! extended (R25, row 2a-2): a follow-up turn on the episode call's own
//! conversation (`distill::Distiller::appraise`), run by `mecha distill`.
//!
//! **One appraisal per session.** The write door reads the ledger under its
//! lock and refuses a second record for a session already on it
//! ([`Recorded::AlreadyOnRecord`]) — the distill ledger and the graph push
//! can each fail after an appraisal was written, and a re-run must not
//! append the same session twice.
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
use crate::message::{Message, Role};
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
/// Grounded claims kept per appraisal; grounded claims past it are dropped
/// as `over_cap`. Every offered claim is grounded first.
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

/// The owner act the appraiser expects next time in this situation — a
/// closed set beside the prose prediction, drawn from R16's table of acts
/// the owner already performs, so row 2b-2 can score a prediction against
/// the recorded act with no model deciding (R27; the owner's ruling of
/// 2026-09-25). Scoring is 2b-2's; this is only the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedAct {
    /// A draft released as written.
    ReleasedUnchanged,
    /// A draft the owner edited before it went.
    Edited,
    /// A draft the owner rejected.
    Rejected,
    /// A task or workflow the owner closed.
    Closed,
    /// A closure the owner undid.
    Reopened,
    /// The owner does nothing with the output.
    NoAct,
    /// A word a newer build wrote.
    #[serde(other)]
    Unknown,
}

impl ExpectedAct {
    /// The six words a producer may write, in R16's order.
    pub const ALL: [ExpectedAct; 6] = [
        ExpectedAct::ReleasedUnchanged,
        ExpectedAct::Edited,
        ExpectedAct::Rejected,
        ExpectedAct::Closed,
        ExpectedAct::Reopened,
        ExpectedAct::NoAct,
    ];

    pub fn wire(self) -> &'static str {
        match self {
            ExpectedAct::ReleasedUnchanged => "released_unchanged",
            ExpectedAct::Edited => "edited",
            ExpectedAct::Rejected => "rejected",
            ExpectedAct::Closed => "closed",
            ExpectedAct::Reopened => "reopened",
            ExpectedAct::NoAct => "no_act",
            ExpectedAct::Unknown => "unknown",
        }
    }

    /// A producer's word: one of [`Self::ALL`], or nothing — a word outside
    /// the set predicts nothing structurally, and is never stored as
    /// `unknown` (that variant is for a newer build's row).
    pub fn parse(word: &str) -> Option<ExpectedAct> {
        let word = word.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        ExpectedAct::ALL.into_iter().find(|a| a.wire() == word)
    }
}

/// `expected_act` from the file, failing soft: absent or null is none, a
/// word this build cannot read is [`ExpectedAct::Unknown`], and a
/// non-string is unknown too rather than a failed row.
fn de_expected_act<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<ExpectedAct>, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(serde_json::from_value(v).unwrap_or(ExpectedAct::Unknown)),
    })
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
    /// The owner act expected next time, from R16's closed set — what 2b-2
    /// scores the prediction by.
    pub expected_act: Option<ExpectedAct>,
    /// Goal words the producer could not read as a pointer at all — counted
    /// on the record with the ones that did not resolve.
    pub unreadable_goals: usize,
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
    /// When the transcript was last written, taken as the read happened —
    /// the session's end as appraised (R37's window opens here). `None`
    /// where the file system could not say.
    ended_at: Option<DateTime<Utc>>,
}

impl SessionEvidence {
    /// Read a transcript file **once**: the one-pass
    /// [`crate::session::Session::parse`] for provenance, anchor and run
    /// config, and [`crate::session::Session::messages_ever`] over the same
    /// bytes for the referents — what compaction later evicted was still
    /// received. One read, because two reads of a session still being
    /// appended to are two snapshots: provenance from the first and
    /// referents from the second would stamp clean a record whose packet
    /// holds an untrusted result the first never covered (found on review
    /// of #308). There is no constructor taking a caller's message list, for
    /// the same reason there is none taking a caller's taint.
    pub fn read(path: &Path) -> Result<SessionEvidence> {
        Ok(SessionEvidence::read_with_transcript(path)?.1)
    }

    /// [`Self::read`], handing back the parsed transcript it read the
    /// evidence from — for a producer that must show a model the same
    /// snapshot the record's provenance describes. `mecha distill` renders
    /// the transcript the appraiser reads from this one read: a second read
    /// of a session still being appended to could show the model an
    /// untrusted result the evidence never covered, and the record would be
    /// stamped clean over it (the #308 review's two-snapshot hole, one layer
    /// out).
    pub fn read_with_transcript(
        path: &Path,
    ) -> Result<(crate::session::Transcript, SessionEvidence)> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        // After the read, so it is no earlier than the last append the read
        // saw: an append-only transcript's last write is the session's end.
        let ended_at = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .map(DateTime::<Utc>::from);
        let transcript = crate::session::Session::parse(path, &text)?;
        let ever = crate::session::Session::messages_ever(&text);
        let mut evidence = SessionEvidence::of(&transcript, &ever);
        evidence.ended_at = ended_at;
        Ok((transcript, evidence))
    }

    /// Both views are of one text, read once by [`Self::read`].
    fn of(transcript: &crate::session::Transcript, ever: &[Message]) -> SessionEvidence {
        // Taint only grows, so the checkpoint covering the last message
        // covers the session; none after it is unknown, which is untrusted.
        let messages = transcript.convo.messages.len();
        let taint = (messages > 0)
            .then(|| transcript.taint_timeline.covering(messages - 1))
            .flatten();
        // The run's situation, from the last run record's matched keys —
        // the goal included, so a reader keyed on the same situation and
        // goal (I2) has the key the block was matched toward — `None` when
        // no config was recorded: unknown, never the empty-keyed scope,
        // which is standing and would match every run.
        let situation = transcript.configs.last().map(|c| {
            Situation::of_run(&c.tools, c.rules_workspace.as_deref())
                .on(c.rules_surface)
                .toward(c.rules_goal.clone())
        });
        SessionEvidence {
            session_id: transcript.meta.id.clone(),
            taint,
            origin: classify_origin(taint),
            anchor: transcript.convo.goal_anchor.clone(),
            situation,
            packet: packet(ever),
            ended_at: None,
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

    /// Every referent as `(id, source, text)` — the id a claim cites, the
    /// tool name or `owner`, and the whole text containment is checked
    /// against. What the appraiser is shown to quote from, so what it
    /// quotes and what the door checks are the same string.
    pub fn referents(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.packet
            .iter()
            .map(|e| (e.id.as_str(), e.source.as_str(), e.text.as_str()))
    }

    /// The session's last goal anchor, as the record will carry it.
    pub fn anchor(&self) -> Option<&GoalRef> {
        self.anchor.as_ref()
    }

    /// The run's situation, as the record will carry it; `None` is unknown.
    pub fn situation(&self) -> Option<&Situation> {
        self.situation.as_ref()
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
        if message.role != Role::User {
            continue;
        }
        // `agent::owner_text` — the one definition of "the owner's own
        // words in this message" (harness messages and every harness voice,
        // the calendar reference included, left out), shared with the
        // learning locators. A referent is what `admit` does literal
        // containment against, so a second spelling here would refuse a
        // quote the renderer showed as the model's fabrication (found on
        // review of #308).
        let owner = crate::agent::owner_text(message);
        if owner.trim().is_empty() {
            continue;
        }
        out.push(crate::grounding::Evidence {
            id: Pointer::Turn(index).to_string(),
            source: "owner".into(),
            text: owner,
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
    /// The owner act expected next time (R16's set) — the prediction's
    /// structural half, for 2b-2's scorer. Absent on a row from before the
    /// field, or when the appraiser named none; a word this build cannot
    /// read loads as [`ExpectedAct::Unknown`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_expected_act"
    )]
    pub expected_act: Option<ExpectedAct>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goal_hypotheses: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lessons: Vec<String>,
    /// Judgments whose goal did not resolve against the stores that mint
    /// goal pointers (`distill::KnownPointers`) — or was no pointer at all —
    /// and so carry no goal. The judgment is kept; the reference is not.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub goals_unresolved: usize,
    /// A bound cut something: the interpretation, the prediction, a
    /// hypothesis or lesson, the number of either, or a judgment's support
    /// list (a repeated index). Claims are never cut — one over a bound is
    /// dropped and counted in `grounding`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clipped: bool,
    /// When the appraised session ended — the transcript's last write as
    /// the appraiser read it. R37's no-act window opens here. Absent on a
    /// row written before the field; a reader then falls back to `at`,
    /// which is later, so the window can only close later, never early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_ended_at: Option<DateTime<Utc>>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
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

    /// Ground `draft` against `evidence`, resolve its judgments' goals
    /// against `known`, and seal it. `None` when the draft has no
    /// interpretation to keep.
    fn seal(
        evidence: &SessionEvidence,
        draft: Draft,
        model: &str,
        known: &crate::distill::KnownPointers,
    ) -> Option<TextAppraisal> {
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
            match ground(&claim, &evidence.packet) {
                // The cap is on what is kept, not on the offered position:
                // ungrounded claims at the head of a draft must not crowd out
                // grounded ones behind them and be reported as `over_cap`
                // instead of the grounding failures they are (review of #308).
                Ok(()) if claims.len() >= MAX_CLAIMS => drop("over_cap".into()),
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
        let mut goals_unresolved = draft.unreadable_goals;
        let judgments = draft
            .judgments
            .into_iter()
            .take(MAX_JUDGMENTS)
            .map(|j| {
                // A goal crosses into the record only if a store that mints
                // goal pointers holds it — the board, the charter, the
                // trigger and front-door stores — the resolution `distill`
                // applies before a pointer rides on an episode. The model
                // writes the reference; a token is not a pointer until a
                // store says so.
                let goal = j.goal.and_then(|g| {
                    let resolved = known.resolve(&g);
                    if resolved.is_none() {
                        goals_unresolved += 1;
                    }
                    resolved
                });
                // Support, deduplicated in the order written and capped at
                // what can be kept: a repeated index is one claim, not
                // several, and the cut is flagged like every other bound
                // (review of #308). Then renumbered to the claims kept —
                // an index to a dropped claim is grounding's loss, already
                // counted there, not a cut.
                let mut seen = std::collections::BTreeSet::new();
                let mut because = Vec::new();
                for i in j.because {
                    if !seen.insert(i) {
                        clipped = true;
                        continue;
                    }
                    if let Some(k) = kept_at.get(&i).copied() {
                        if because.len() >= MAX_CLAIMS {
                            clipped = true;
                            break;
                        }
                        because.push(k);
                    }
                }
                Judgment {
                    goal,
                    bearing: j.bearing,
                    because,
                }
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
            // The structural half is kept even when the prose prediction is
            // empty: an expected act is a prediction on its own.
            expected_act: draft.expected_act.filter(|a| *a != ExpectedAct::Unknown),
            goal_hypotheses,
            lessons,
            goals_unresolved,
            clipped,
            session_ended_at: evidence.ended_at,
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
    // A blank entry dropped is a loss too, and every loss is flagged.
    if kept.len() > count || kept.len() < items.len() {
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

/// How many past appraisals the appraiser is shown (I1, I2: "up to three
/// clean appraisals of the same situation and goal").
pub const PAST_SHOWN: usize = 3;

impl CleanRead {
    /// The clean appraisals of the same situation and goal as `evidence` —
    /// newest first, at most `n`, never the session's own. Only a [`Clean`]
    /// can be returned, so a tainted appraisal cannot reach the appraiser
    /// of a later session through here, whatever its situation.
    ///
    /// **Same situation and goal** is the same region key
    /// ([`Situation::key`]: the scope's tools, workspace, surface and — since
    /// 2c-1 — the goal the run's block was matched toward,
    /// `RunConfig::rules_goal`), compared exactly, so nothing widens: a run
    /// that presented no goal matches only records that presented none,
    /// never a goal-scoped one, and a goal-scoped run matches only its own
    /// goal. What cannot be keyed matches nothing: an unknown situation on
    /// either side (unknown is never "everywhere"), and a goal or surface
    /// this build cannot name (`GoalKey::Unread`, `surface_unread`) — kept
    /// verbatim in the key, and still never equal to anything.
    pub fn same_situation_and_goal(&self, evidence: &SessionEvidence, n: usize) -> Vec<&Clean> {
        self.keyed_as(
            evidence.situation.as_ref(),
            Some(evidence.session_id.as_str()),
            n,
        )
    }

    fn keyed_as(&self, here: Option<&Situation>, not: Option<&str>, n: usize) -> Vec<&Clean> {
        newest_keyed(self.appraisals.iter(), here, not, n)
    }
}

/// The region key a record is compared on, or `None` where it cannot be
/// keyed: an unknown situation, or a surface or goal this build cannot name.
fn region_key(s: Option<&Situation>) -> Option<String> {
    let s = s?;
    let unnamed =
        s.surface_unread.is_some() || matches!(s.goal, Some(crate::situation::GoalKey::Unread(_)));
    (!unnamed).then(|| s.key())
}

/// The newest `n` of `rows` whose region key is `here`'s, never session
/// `not`'s. One selection for the appraiser's door and the run's.
fn newest_keyed<'a>(
    rows: impl Iterator<Item = &'a Clean>,
    here: Option<&Situation>,
    not: Option<&str>,
    n: usize,
) -> Vec<&'a Clean> {
    let Some(here) = region_key(here) else {
        return Vec::new();
    };
    let mut out: Vec<&Clean> = rows
        .filter(|c| not.is_none_or(|id| c.session_id != id))
        .filter(|c| region_key(c.situation.as_ref()).as_ref() == Some(&here))
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.at));
    out.truncate(n);
    out
}

/// What `goal_context` may serve a run (I2, built as 2c-2): up to
/// [`PAST_SHOWN`] clean appraisals of the situation the run's rules block
/// was matched against, the goal included. Only a [`Clean`] is held, so a
/// tainted appraisal cannot be served whatever the caller loaded.
///
/// **Selected in two steps**, because the tool set a run records is not the
/// one its block was matched against: a front-end takes tools off the
/// registry after `setup::build` (`tasks work` and a question continuation
/// withhold `kg_task_update`), and the run record — which a later appraisal
/// of this run is keyed on — reads the registry as it is when the run
/// starts. So `select` keeps the clean records that agree on every key but
/// the tools, and [`Self::for_registry`], called by the loop at run start
/// with the registry it actually carries, picks the newest that agree on
/// the tools too. Without the second step a delegated task's retrieval
/// never matched a past run of the same task (found building it).
///
/// No session is excluded, unlike the appraiser's door: a fresh run has no
/// appraisal of its own, and a resumed session appraised earlier may be
/// served its own, which is of the same situation and goal by construction.
#[derive(Debug, Clone, Default)]
pub struct PastAppraisals {
    goal: Option<GoalRef>,
    run: Situation,
    pool: Vec<Clean>,
    served: Vec<Clean>,
    unread: Option<String>,
}

impl PastAppraisals {
    /// The clean records toward `run` on every key but the tools, and the
    /// first selection against `run`'s own tools.
    pub fn select(read: CleanRead, run: &Situation) -> PastAppraisals {
        let untooled = |s: &Situation| Situation {
            tools: Vec::new(),
            ..s.clone()
        };
        let here = region_key(Some(&untooled(run)));
        let pool: Vec<Clean> = read
            .appraisals
            .into_iter()
            .filter(|c| {
                here.is_some() && region_key(c.situation.as_ref().map(untooled).as_ref()) == here
            })
            .collect();
        let mut out = PastAppraisals {
            goal: run
                .goal
                .as_ref()
                .and_then(crate::situation::GoalKey::named)
                .cloned(),
            run: run.clone(),
            pool,
            served: Vec::new(),
            unread: None,
        };
        let tools = run.tools.clone();
        out.for_registry(&tools);
        out
    }

    /// The store could not be read: nothing is served, and the answer says
    /// why rather than reading as "no past appraisal".
    pub fn unread(why: String, run: &Situation) -> PastAppraisals {
        PastAppraisals {
            goal: run
                .goal
                .as_ref()
                .and_then(crate::situation::GoalKey::named)
                .cloned(),
            run: run.clone(),
            unread: Some(why),
            ..PastAppraisals::default()
        }
    }

    /// Re-select against the tools the run carries — the registry the run
    /// record will name.
    pub fn for_registry(&mut self, tools: &[String]) {
        let run = Situation {
            tools: tools.to_vec(),
            ..self.run.clone()
        };
        self.served = newest_keyed(self.pool.iter(), Some(&run), None, PAST_SHOWN)
            .into_iter()
            .cloned()
            .collect();
    }

    /// The goal the selection is toward — the run's matched goal. Served
    /// only to a request toward it.
    pub fn goal(&self) -> Option<&GoalRef> {
        self.goal.as_ref()
    }

    pub fn served(&self) -> &[Clean] {
        &self.served
    }

    pub fn unread_reason(&self) -> Option<&str> {
        self.unread.as_deref()
    }
}

/// A [`CleanRead`] of `rows` through the same admission the store's door
/// uses ([`Clean::admit`]) — for a test in another module that needs a
/// `Clean`, which it cannot otherwise build. A tainted row is withheld here
/// exactly as `AppraisalStore::clean` withholds it.
#[cfg(test)]
pub(crate) fn clean_read_of(rows: Vec<TextAppraisal>) -> CleanRead {
    let mut read = CleanRead::default();
    for row in rows {
        match Clean::admit(row) {
            Some(c) => read.appraisals.push(c),
            None => read.withheld += 1,
        }
    }
    read
}

/// A stored appraisal of session `session` in `situation`, clean or
/// tainted — the shape `AppraisalStore::record` writes, for other modules'
/// tests.
#[cfg(test)]
pub(crate) fn test_row(
    session: &str,
    situation: &Situation,
    clean: bool,
    at: &str,
) -> TextAppraisal {
    serde_json::from_value(serde_json::json!({
        "id": format!("apr-{session}"),
        "at": at,
        "session_id": session,
        "origin": if clean { "clean" } else { "untrusted" },
        "taint": {"private": true, "untrusted": !clean},
        "situation": situation,
        "model": "local-model",
        "interpretation": format!("In {session} the owner wanted the date quoted from the mail."),
        "prediction": "Next time the owner will want the date confirmed.",
        "lessons": ["Quote the date from the mail when passing it on."],
    }))
    .unwrap()
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
    /// The session already has an appraisal on record, `id`. Nothing was
    /// written: one appraisal per session.
    AlreadyOnRecord { id: String },
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

    /// The one write door: ground `draft` against `evidence`, resolve its
    /// goals against `known`, stamp the session's recorded provenance, and
    /// append under the lock, synced — unless the session already has an
    /// appraisal on record, checked under the same lock, which is refused
    /// as [`Recorded::AlreadyOnRecord`]. A tainted session's appraisal is
    /// written — it is the owner's — and the clean door will never serve it.
    /// An I/O failure is an `Err`, never a quiet no-op: a caller that paid a
    /// model call must hear it was lost.
    pub fn record(
        &self,
        evidence: &SessionEvidence,
        draft: Draft,
        model: &str,
        known: &crate::distill::KnownPointers,
    ) -> Result<Recorded> {
        let Some(record) = TextAppraisal::seal(evidence, draft, model, known) else {
            return Ok(Recorded::Empty);
        };
        use std::io::Write;
        let _lock = self.lock()?;
        if let Some(id) = self.on_record_unlocked(&record.session_id)? {
            return Ok(Recorded::AlreadyOnRecord { id });
        }
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

    /// The id of the appraisal already on record for `session_id`, if any —
    /// what a producer asks before it pays for a model call whose record the
    /// door would refuse. A read that fails is an `Err`: "could not tell"
    /// is not "none on record".
    pub fn on_record(&self, session_id: &str) -> Result<Option<String>> {
        self.on_record_unlocked(session_id)
    }

    /// The same question, for [`Self::record`] under its lock. A torn line
    /// is skipped as `for_owner` skips it: it names no session this build
    /// can read, and refusing every write because of one would stop the
    /// store for good.
    fn on_record_unlocked(&self, session_id: &str) -> Result<Option<String>> {
        if session_id.trim().is_empty() {
            return Ok(None);
        }
        let (rows, _) = self.for_owner()?;
        Ok(rows
            .into_iter()
            .find(|r| r.session_id == session_id)
            .map(|r| r.id))
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
    /// Distinct sessions appraised, counting only rows that name one.
    pub sessions: usize,
    /// Rows that name no session — a sparse or hand-written row. Kept apart
    /// so they never read as one more session.
    pub no_session: usize,
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
    /// Records carrying an expected owner act (the prediction's structural
    /// half, for 2b-2).
    pub with_expected_act: usize,
    /// Judgment goals that did not resolve, across records.
    pub goals_unresolved: usize,
}

impl Summary {
    pub fn of(rows: &[TextAppraisal]) -> Summary {
        let mut s = Summary {
            records: rows.len(),
            ..Summary::default()
        };
        let mut sessions = std::collections::BTreeSet::new();
        for r in rows {
            if r.session_id.trim().is_empty() {
                s.no_session += 1;
            } else {
                sessions.insert(r.session_id.as_str());
            }
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
            if r.expected_act.is_some() {
                s.with_expected_act += 1;
            }
            s.goals_unresolved += r.goals_unresolved;
        }
        s.sessions = sessions.len();
        s
    }
}

// ─── The appraisal's own prediction, scored (X5, R33, R37; row 2b-2) ──────

/// The window before "no act" becomes the act that happened, for an output
/// with no store patience to take — a chat answer, a run that staged
/// nothing, a **workflow** the session worked (the workflow store carries
/// no `doctor::Patience`; kept at the constant for now, the owner's ruling
/// of 2026-09-25), and a **task with no due date**. R37: "the doctor's
/// constant", confirmed by the owner as the outbox's 48h. A task *with* a
/// due date takes its window from the date instead (R37 refined; see
/// [`observe`]).
pub const NO_STORE_PATIENCE_HOURS: i64 = 48;

/// What the owner did with a session's output, as the stores that record
/// the owner's acts say — read by the harness, never by a model.
#[derive(Debug, Clone, PartialEq)]
pub enum ObservedAct {
    /// The owner's first act on the output within the window.
    Act { act: ExpectedAct, at: DateTime<Utc> },
    /// The window closed with no act: "no act" is the act that happened
    /// (R37).
    NoAct { window_closed_at: DateTime<Utc> },
    /// The window is still open and no act has arrived.
    Pending { closes_at: DateTime<Utc> },
    /// The output is a task and this caller did not read the board, so its
    /// due date — the window — is not known here. Not unknown: `mecha
    /// distill` reads the board and scores it. Nothing is written.
    NeedsBoard,
    /// Something the answer depends on could not be read — an act store,
    /// the charter the patience comes from, a timestamp. **Never "no
    /// act"**: an act the harness could not see is not an act that did not
    /// happen.
    Unknown { why: String },
}

/// The stores the owner's acts on a session's output live in, as a caller
/// read them. A flag set means that store is unknown rather than empty.
#[derive(Debug, Clone, Copy, Default)]
pub struct OwnerActs<'a> {
    /// Every outbox item; filtered to the session inside.
    pub drafts: &'a [crate::outbox::OutboxItem],
    pub outbox_unreadable: bool,
    /// The closure store's standing transitions.
    pub closures: &'a [crate::closure::Transition],
    pub closures_unreadable: bool,
    pub workflows: &'a [crate::workflow::Workflow],
    pub workflows_unreadable: bool,
    /// The charter, for the patience of the outbox's line.
    pub charter: Option<&'a crate::charter::Charter>,
    pub charter_unreadable: bool,
    /// The board (`kg_task_list` with closed rows), read by the harness —
    /// where a task output's due date comes from (R37 refined).
    pub board: BoardRead<'a>,
    /// The owner's zone (`[agent] timezone`), for a due *date*: "by the
    /// due date" is through the end of that day where the owner is. `None`
    /// reads the day in UTC, the machine's zone.
    pub zone: Option<chrono_tz::Tz>,
}

/// The board as a caller has it.
#[derive(Debug, Clone, Copy, Default)]
pub enum BoardRead<'a> {
    /// This caller did not read it (the read-only readout has no graph
    /// connection).
    #[default]
    NotRead,
    /// The read failed or the answer was not a board: unknown.
    Unreadable,
    /// The answer, `{"items": [...], "truncated": ...}`.
    Read(&'a serde_json::Value),
}

/// When a task's no-act window closes: its `due_at` from the board, as the
/// instant the due day ends where the owner is (or the stated instant, if
/// the row carries a time). `Ok(None)` is a task with no due date — the
/// constant applies. `Err` is unknown: an unreadable board, a row the board
/// does not have, or a date that will not parse.
fn task_due(
    task: &str,
    board: &BoardRead<'_>,
    zone: Option<chrono_tz::Tz>,
) -> std::result::Result<Option<DateTime<Utc>>, String> {
    let board = match board {
        BoardRead::NotRead => return Err("not read".into()),
        BoardRead::Unreadable => return Err("the board could not be read".into()),
        BoardRead::Read(v) => v,
    };
    let Some(rows) = board["items"].as_array() else {
        return Err("the board's answer carried no task list".into());
    };
    let Some(row) = rows.iter().find(|r| r["id"].as_str() == Some(task)) else {
        return Err(format!("the board has no row for task {task}"));
    };
    let raw = match &row["due_at"] {
        serde_json::Value::Null => return Ok(None),
        serde_json::Value::String(s) if s.trim().is_empty() => return Ok(None),
        serde_json::Value::String(s) => s.trim().to_string(),
        _ => return Err(format!("task {task}'s due date is not a date")),
    };
    if let Ok(at) = DateTime::parse_from_rfc3339(&raw) {
        return Ok(Some(at.with_timezone(&Utc)));
    }
    let day = raw
        .get(..10)
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .ok_or_else(|| format!("task {task}'s due date {raw:?} will not parse"))?;
    let next = day
        .succ_opt()
        .ok_or_else(|| format!("task {task}'s due date is out of range"))?
        .and_hms_opt(0, 0, 0)
        .expect("midnight exists");
    let end = match zone {
        Some(tz) => next
            .and_local_timezone(tz)
            .earliest()
            .map(|t| t.with_timezone(&Utc)),
        None => Some(next.and_utc()),
    };
    end.map(Some)
        .ok_or_else(|| format!("task {task}'s due day has no end in the owner's zone"))
}

/// R37's window for one session's output: the patience of the store the
/// output waits in — the outbox's line where the charter has one, else the
/// doctor's constant — or [`NO_STORE_PATIENCE_HOURS`] for an output with
/// no store. `Err` when the patience cannot be read.
fn patience_for(
    has_drafts: bool,
    acts: &OwnerActs<'_>,
) -> std::result::Result<chrono::Duration, String> {
    if !has_drafts {
        return Ok(chrono::Duration::hours(NO_STORE_PATIENCE_HOURS));
    }
    if acts.charter_unreadable {
        return Err("the charter could not be read, so the outbox's patience is unknown".into());
    }
    crate::doctor::Patience::for_store(acts.charter, crate::charter::SensorKind::OutboxAge)
        .map(|p| p.after)
        .ok_or_else(|| "the outbox has no patience".into())
}

/// The owner's act on `session_id`'s output, within R37's window opened at
/// `ended_at`, as of `now`.
///
/// The acts, R16's set (R33): a model-authored draft released unchanged or
/// after the owner's edit, or rejected; a task the session worked closed or
/// reopened by the owner (the closure record's `sessions`); a workflow that
/// tracked the session closed, reopened or cancelled — a cancel is the
/// owner declining the work, read as `rejected`. **The first act by time
/// is the act**: the owner's first reaction to the output, whatever came
/// after it. An act after the window closed is not the act (R37: only an
/// act that arrives before the window closes).
pub fn observe(
    session_id: &str,
    anchor: Option<&GoalRef>,
    ended_at: DateTime<Utc>,
    acts: &OwnerActs<'_>,
    now: DateTime<Utc>,
) -> ObservedAct {
    let unknown = |why: &str| ObservedAct::Unknown {
        why: why.to_string(),
    };
    // A row naming no session has no output to read an act off: "nothing
    // matched" would read as the owner doing nothing.
    if session_id.trim().is_empty() {
        return unknown("the appraisal names no session");
    }
    if acts.outbox_unreadable {
        return unknown("the outbox could not be fully read");
    }
    if acts.closures_unreadable {
        return unknown("the closure store could not be fully read");
    }
    if acts.workflows_unreadable {
        return unknown("the workflow store could not be read");
    }
    let mut seen: Vec<(DateTime<Utc>, ExpectedAct)> = Vec::new();
    let drafts: Vec<&crate::outbox::OutboxItem> = acts
        .drafts
        .iter()
        .filter(|d| d.session_id.as_deref() == Some(session_id))
        .filter(|d| d.author() == crate::outbox::Author::Model)
        .collect();
    for d in &drafts {
        let act = match d.status.as_str() {
            "sent" if d.edited() => ExpectedAct::Edited,
            "sent" => ExpectedAct::ReleasedUnchanged,
            "rejected" => ExpectedAct::Rejected,
            _ => continue,
        };
        let Some(at) = d
            .resolved_at
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
        else {
            return unknown("a resolved draft carries no readable time");
        };
        seen.push((at, act));
    }
    for c in acts.closures {
        // An actor this build cannot read is not taken as the owner's hand:
        // the act is not established to be the owner's, so it is not one,
        // the direction `observe` fails closed toward everywhere.
        if !c.sessions.iter().any(|s| s == session_id)
            || !matches!(
                c.actor,
                crate::closure::Actor::Owner | crate::closure::Actor::OwnerApproved
            )
        {
            continue;
        }
        match c.kind {
            crate::closure::Move::Close => seen.push((c.at, ExpectedAct::Closed)),
            crate::closure::Move::Reopen => seen.push((c.at, ExpectedAct::Reopened)),
            // The owner acted on this session's task and this build cannot
            // read which way: an act was seen and not read, which is
            // unknown — never "no act" (R37; found on review of #324).
            crate::closure::Move::Unknown => {
                return unknown(
                    "a closure naming this session records a move this build cannot read",
                )
            }
        }
    }
    for w in acts.workflows {
        for d in w.owner_dispositions() {
            if d.session.as_deref() != Some(session_id) {
                continue;
            }
            match d.kind {
                crate::workflow::Disposition::Closed => seen.push((d.at, ExpectedAct::Closed)),
                crate::workflow::Disposition::Cancelled => seen.push((d.at, ExpectedAct::Rejected)),
            }
            if let Some(at) = d.reopened_at {
                seen.push((at, ExpectedAct::Reopened));
            }
        }
    }
    // The output's window (R37, refined by the owner 2026-09-25). A session
    // that staged drafts waits in the outbox: its patience. Otherwise, a
    // task the session worked — its anchor, else the task a closure naming
    // the session moved — runs to the task's due date where the board row
    // has one, else the constant. A workflow, or nothing, is the constant.
    let task = match anchor {
        Some(GoalRef::Task(id)) => Some(id.clone()),
        _ => acts
            .closures
            .iter()
            .find(|c| c.sessions.iter().any(|s| s == session_id))
            .map(|c| c.task.clone()),
    };
    let constant = ended_at + chrono::Duration::hours(NO_STORE_PATIENCE_HOURS);
    let closes_at = if !drafts.is_empty() {
        match patience_for(true, acts) {
            Ok(p) => ended_at + p,
            Err(why) => return ObservedAct::Unknown { why },
        }
    } else if let Some(task) = task {
        match task_due(&task, &acts.board, acts.zone) {
            // A due date still ahead at the session's end is the window.
            Ok(Some(due)) if due > ended_at => due,
            // Already past when the session ended (or none): the output is
            // still waiting on the owner's reaction, and closing the window
            // at once would score every overdue task's review as "no act"
            // whatever the owner then did — the constant instead.
            Ok(_) => constant,
            Err(_) if matches!(acts.board, BoardRead::NotRead) => return ObservedAct::NeedsBoard,
            Err(why) => return ObservedAct::Unknown { why },
        }
    } else {
        constant
    };
    seen.sort_by_key(|(at, _)| *at);
    if let Some((at, act)) = seen.into_iter().find(|(at, _)| *at <= closes_at) {
        return ObservedAct::Act { act, at };
    }
    if now >= closes_at {
        ObservedAct::NoAct {
            window_closed_at: closes_at,
        }
    } else {
        ObservedAct::Pending { closes_at }
    }
}

/// One appraisal's prediction, scored: its expected act against the act
/// that happened. Written once per appraisal, when the act resolves, and
/// never rewritten — a later charter edit that moves the patience does not
/// re-score what was scored.
///
/// **A miss is a surprise** (X5): recorded here, as `surprise`, for 2e-6's
/// replay priority and 2d-1's surprise decision points to read. Nothing
/// ranks on it yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Score {
    pub id: String,
    pub scored_at: DateTime<Utc>,
    #[serde(default)]
    pub appraisal_id: String,
    #[serde(default)]
    pub session_id: String,
    /// The appraisal's `expected_act` (R33).
    #[serde(deserialize_with = "de_act")]
    pub expected: ExpectedAct,
    /// What happened: an act from R16's set, or `no_act` once the window
    /// closed (R37).
    #[serde(deserialize_with = "de_act")]
    pub actual: ExpectedAct,
    /// When the act happened; `None` for `no_act`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acted_at: Option<DateTime<Utc>>,
    /// When the window closed, or would have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_closed_at: Option<DateTime<Utc>>,
    /// Derived: `expected == actual`. Stored so a reader never re-derives
    /// it differently.
    #[serde(default)]
    pub hit: bool,
    /// Derived: a miss. The surprise X5 names.
    #[serde(default)]
    pub surprise: bool,
    /// Whether the appraisal scored is served by the clean door — a reader
    /// that acts (replay priority, 2e-6) must take only clean ones.
    #[serde(default)]
    pub clean: bool,
    /// The appraisal's situation and anchor, so a reader can find "the same
    /// situation and goal" without joining back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub situation: Option<Situation>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::goal::de_lenient"
    )]
    pub anchor: Option<GoalRef>,
}

/// An act word from the file, failing soft to `unknown`.
fn de_act<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ExpectedAct, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or(ExpectedAct::Unknown))
}

/// What scoring one appraisal found.
#[derive(Debug, Clone, PartialEq)]
pub enum Scored {
    /// Scored and written.
    Written(Box<Score>),
    /// Already scored; nothing written.
    AlreadyScored,
    /// The appraisal predicted no act structurally: nothing to score.
    NoExpectation,
    /// The window is open and nothing has happened yet.
    Pending { closes_at: DateTime<Utc> },
    /// The answer could not be read; nothing written.
    Unknown { why: String },
    /// A task output, and the board was not read by this caller.
    NeedsBoard,
}

/// Coverage of the appraisals' predictions: how many are scored, how many
/// wait, and a hit rate only over scores that exist.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ScoreSummary {
    pub appraisals: usize,
    /// Appraisals carrying a readable expected act.
    pub with_expectation: usize,
    pub scored: usize,
    pub hits: usize,
    /// Misses — surprises.
    pub surprises: usize,
    /// Of the surprises, those on clean appraisals.
    pub clean_surprises: usize,
    /// Expectations whose window is still open and no act has arrived.
    pub pending: usize,
    /// Expectations that have resolved — an act, or the window closed —
    /// and are not yet written: the next `mecha distill` pass scores them.
    pub resolved_unwritten: usize,
    /// Expectations whose answer could not be read.
    pub unknown: usize,
    /// Task outputs this reader could not window, because it read no board
    /// — the read-only readout; `mecha distill` reads it and scores them.
    pub board_not_read: usize,
    /// `hits / scored`; `None` over no scores.
    pub hit_rate: Option<f64>,
    /// Score lines that could not be read.
    pub skipped: usize,
    /// Appraisal lines that could not be read — each may have carried an
    /// expectation, so every count above is a floor when this is not zero.
    pub appraisals_unreadable: usize,
}

impl AppraisalStore {
    fn scores_ledger(&self) -> PathBuf {
        self.root.join("scores.jsonl")
    }

    /// Every score, oldest first, and how many lines were skipped. A
    /// missing file is no scores; one that cannot be read is an `Err`.
    pub fn scores(&self) -> Result<(Vec<Score>, usize)> {
        let path = self.scores_ledger();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut out = Vec::new();
        let mut skipped = 0usize;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str(line) {
                Ok(s) => out.push(s),
                Err(_) => skipped += 1,
            }
        }
        Ok((out, skipped))
    }

    /// Score one appraisal against what the owner did, and write the score
    /// if the act has resolved — once per appraisal, checked under the
    /// store's lock. Pending and unknown write nothing.
    pub fn score(
        &self,
        appraisal: &TextAppraisal,
        acts: &OwnerActs<'_>,
        now: DateTime<Utc>,
    ) -> Result<Scored> {
        let Some(expected) = appraisal
            .expected_act
            .filter(|a| *a != ExpectedAct::Unknown)
        else {
            return Ok(Scored::NoExpectation);
        };
        let ended_at = appraisal.session_ended_at.unwrap_or(appraisal.at);
        let (actual, acted_at, window_closed_at) = match observe(
            &appraisal.session_id,
            appraisal.anchor.as_ref(),
            ended_at,
            acts,
            now,
        ) {
            ObservedAct::Act { act, at } => (act, Some(at), None),
            ObservedAct::NoAct { window_closed_at } => {
                (ExpectedAct::NoAct, None, Some(window_closed_at))
            }
            ObservedAct::Pending { closes_at } => return Ok(Scored::Pending { closes_at }),
            ObservedAct::Unknown { why } => return Ok(Scored::Unknown { why }),
            ObservedAct::NeedsBoard => return Ok(Scored::NeedsBoard),
        };
        let hit = expected == actual;
        let score = Score {
            id: format!("scr-{}", uuid::Uuid::new_v4()),
            scored_at: now,
            appraisal_id: appraisal.id.clone(),
            session_id: appraisal.session_id.clone(),
            expected,
            actual,
            acted_at,
            window_closed_at,
            hit,
            surprise: !hit,
            clean: appraisal.is_clean(),
            situation: appraisal.situation.clone(),
            anchor: appraisal.anchor.clone(),
        };
        use std::io::Write;
        let _lock = self.lock()?;
        let (existing, _) = self.scores()?;
        if existing.iter().any(|s| s.appraisal_id == appraisal.id) {
            return Ok(Scored::AlreadyScored);
        }
        let path = self.scores_ledger();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        let mut line = serde_json::to_string(&score)?;
        line.push('\n');
        file.write_all(line.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        file.sync_data()
            .with_context(|| format!("syncing {}", path.display()))?;
        Ok(Scored::Written(Box::new(score)))
    }

    /// Score every appraisal whose act has resolved and is not yet scored —
    /// what `mecha distill` runs each pass, no model call. Returns the
    /// coverage after the pass.
    pub fn score_due(&self, acts: &OwnerActs<'_>, now: DateTime<Utc>) -> Result<ScoreSummary> {
        let (rows, _) = self.for_owner()?;
        for row in &rows {
            self.score(row, acts, now)?;
        }
        self.score_summary(acts, now)
    }

    /// Coverage, read-only: the ledger's scores, and for every expectation
    /// not yet scored, whether its window is open or its answer unknown.
    pub fn score_summary(&self, acts: &OwnerActs<'_>, now: DateTime<Utc>) -> Result<ScoreSummary> {
        let (rows, appraisals_unreadable) = self.for_owner()?;
        let (scores, skipped) = self.scores()?;
        let mut s = ScoreSummary {
            appraisals: rows.len(),
            skipped,
            appraisals_unreadable,
            ..ScoreSummary::default()
        };
        for row in &rows {
            if !row.expected_act.is_some_and(|a| a != ExpectedAct::Unknown) {
                continue;
            }
            s.with_expectation += 1;
            if let Some(score) = scores.iter().find(|x| x.appraisal_id == row.id) {
                s.scored += 1;
                if score.hit {
                    s.hits += 1;
                } else {
                    s.surprises += 1;
                    if score.clean {
                        s.clean_surprises += 1;
                    }
                }
                continue;
            }
            let ended_at = row.session_ended_at.unwrap_or(row.at);
            match observe(&row.session_id, row.anchor.as_ref(), ended_at, acts, now) {
                ObservedAct::Unknown { .. } => s.unknown += 1,
                ObservedAct::NeedsBoard => s.board_not_read += 1,
                ObservedAct::Pending { .. } => s.pending += 1,
                ObservedAct::Act { .. } | ObservedAct::NoAct { .. } => s.resolved_unwritten += 1,
            }
        }
        s.hit_rate = (s.scored > 0).then(|| s.hits as f64 / s.scored as f64);
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compact::SUPERSEDED_MARKER;
    use crate::message::Block;
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
                rules_goal: Some(crate::situation::GoalKey::Named(GoalRef::Task(
                    "t-budget".into(),
                ))),
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
            expected_act: Some(ExpectedAct::ReleasedUnchanged),
            unreadable_goals: 0,
            goal_hypotheses: vec!["The owner wants colleagues kept informed.".into()],
            lessons: vec!["Quote the date from the mail when passing it on.".into()],
        }
    }

    /// The board holds the fixture's task; nothing else resolves.
    fn known() -> crate::distill::KnownPointers {
        crate::distill::KnownPointers::from_board(&json!({"items": [{"id": "t-budget"}]}))
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
        } = store
            .record(&evidence, draft(), "local-model", &known())
            .unwrap()
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
        // The goal the block was matched toward is on the situation, so a
        // reader keyed on the same situation and goal (I2) has the key.
        assert_eq!(
            situation.goal,
            Some(crate::situation::GoalKey::Named(GoalRef::Task(
                "t-budget".into()
            )))
        );
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
                store.record(&evidence, draft(), "m", &known()).unwrap(),
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
        for _ in 0..MAX_CLAIMS {
            d.claims
                .push(claim("Filler", "result:t1", "budget review moved"));
        }
        let offered = d.claims.len();
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        let Recorded::Written { grounding, .. } =
            store.record(&evidence, d, "m", &known()).unwrap()
        else {
            panic!("written");
        };
        // Seven fail grounding; of the fourteen that pass, twelve are kept
        // and the two past the cap are `over_cap` — the failures at the head
        // are counted as failures, not as crowding.
        assert_eq!(grounding.offered, offered);
        assert_eq!(grounding.dropped, offered - MAX_CLAIMS);
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

        // A turn of two owner blocks — a steer beside tool results — is the
        // string the learning locators agree on, byte for byte: a quote is
        // checked by containment against exactly this text.
        let mut steer = Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "t1".into(),
            content: "a.md".into(),
            is_error: false,
        }]);
        steer.content.push(Block::text("only b.md, "));
        steer.content.push(Block::text("and quickly"));
        let got = packet(std::slice::from_ref(&steer));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, crate::agent::owner_text(&steer));
        assert_eq!(got[0].text, "only b.md, and quickly");
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
        // One appraisal per session, so every record below is of a fresh
        // session.
        let fresh =
            || SessionEvidence::read(&session(&root.join("sessions"), clean_taint())).unwrap();
        let evidence = fresh();
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        let empty = Draft {
            interpretation: "   ".into(),
            ..draft()
        };
        assert_eq!(
            store.record(&evidence, empty, "m", &known()).unwrap(),
            Recorded::Empty
        );
        assert!(!store.ledger().exists(), "nothing was written");

        let long = Draft {
            interpretation: "é".repeat(INTERPRETATION_MAX_CHARS + 5),
            goal_hypotheses: vec!["h".into(); MAX_HYPOTHESES + 1],
            ..draft()
        };
        store.record(&fresh(), long, "m", &known()).unwrap();
        let got = &store.for_owner().unwrap().0[0];
        assert!(got.clipped);
        assert_eq!(got.interpretation.chars().count(), INTERPRETATION_MAX_CHARS);
        assert_eq!(got.goal_hypotheses.len(), MAX_HYPOTHESES);

        // Each remaining bound, one draft apiece, so a flag set by one
        // cannot hide another that sets none.
        let cases: Vec<(&str, Draft)> = vec![
            (
                "prediction",
                Draft {
                    prediction: Some("p".repeat(PREDICTION_MAX_CHARS + 1)),
                    ..draft()
                },
            ),
            (
                "lesson length",
                Draft {
                    lessons: vec!["l".repeat(LESSON_MAX_CHARS + 1)],
                    ..draft()
                },
            ),
            (
                "lesson count",
                Draft {
                    lessons: vec!["l".into(); MAX_LESSONS + 1],
                    ..draft()
                },
            ),
            (
                "hypothesis length",
                Draft {
                    goal_hypotheses: vec!["h".repeat(HYPOTHESIS_MAX_CHARS + 1)],
                    ..draft()
                },
            ),
            (
                "a repeated support index",
                Draft {
                    judgments: vec![Judgment {
                        goal: Some(GoalRef::Task("t-budget".into())),
                        bearing: Bearing::Good,
                        because: vec![0, 0, 1, 1, 0],
                    }],
                    ..draft()
                },
            ),
        ];
        for (what, d) in cases {
            store.record(&fresh(), d, "m", &known()).unwrap();
            let rows = store.for_owner().unwrap().0;
            let got = rows.last().unwrap();
            assert!(got.clipped, "{what}");
            assert!(got
                .prediction
                .as_deref()
                .is_none_or(|p| p.chars().count() <= PREDICTION_MAX_CHARS));
            assert!(got.lessons.len() <= MAX_LESSONS, "{what}");
            assert!(got
                .lessons
                .iter()
                .chain(&got.goal_hypotheses)
                .all(|t| t.chars().count() <= LESSON_MAX_CHARS.max(HYPOTHESIS_MAX_CHARS)));
            assert!(
                got.judgments.iter().all(|j| {
                    let mut d = j.because.clone();
                    d.dedup();
                    d == j.because && j.because.len() <= MAX_CLAIMS
                }),
                "{what}: support is deduplicated and capped"
            );
        }
        let repeated = store.for_owner().unwrap().0;
        assert_eq!(
            repeated.last().unwrap().judgments[0].because,
            vec![0, 1],
            "a repeated index is one claim, in the order written"
        );
        // A statement over its ceiling is dropped, never cut, and counted.
        let wordy = Draft {
            claims: vec![claim(
                &"s".repeat(STATEMENT_MAX_CHARS + 1),
                "result:t1",
                "the budget review moved to Thursday",
            )],
            ..draft()
        };
        let Recorded::Written { grounding, .. } =
            store.record(&fresh(), wordy, "m", &known()).unwrap()
        else {
            panic!("written");
        };
        assert_eq!(grounding.dropped_by.get("statement_too_long"), Some(&1));
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
            store.record(&evidence, d, "m", &known()).unwrap();
        }
        let s = Summary::of(&store.for_owner().unwrap().0);
        assert_eq!((s.records, s.sessions, s.clean, s.not_clean), (2, 2, 1, 1));
        assert_eq!(s.no_session, 0);
        let mut sparse = store.for_owner().unwrap().0;
        sparse[0].session_id = String::new();
        let s = Summary::of(&sparse);
        assert_eq!(
            (s.sessions, s.no_session),
            (1, 1),
            "a row naming no session is not one more session"
        );
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

    /// One appraisal per session: a second record of the same session —
    /// a re-run after the graph push failed, a second nightly — is refused
    /// under the lock and appends nothing. Fails on 2a-1's door, which
    /// appended again.
    #[test]
    fn a_second_appraisal_of_a_session_is_refused_and_nothing_appended() {
        let root = temp_root("once");
        let path = session(&root.join("sessions"), clean_taint());
        let evidence = SessionEvidence::read(&path).unwrap();
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        assert_eq!(store.on_record(evidence.session_id()).unwrap(), None);
        let Recorded::Written { id, .. } = store.record(&evidence, draft(), "m", &known()).unwrap()
        else {
            panic!("written");
        };
        assert_eq!(
            store.on_record(evidence.session_id()).unwrap(),
            Some(id.clone())
        );
        // A fresh handle, a fresh read of the transcript: still refused.
        let again = AppraisalStore::open(root.join("appraisals")).unwrap();
        let reread = SessionEvidence::read(&path).unwrap();
        assert_eq!(
            again.record(&reread, draft(), "m", &known()).unwrap(),
            Recorded::AlreadyOnRecord { id }
        );
        let raw = std::fs::read_to_string(store.ledger()).unwrap();
        assert_eq!(raw.lines().count(), 1, "{raw}");
        // Another session still writes.
        let other = SessionEvidence::read(&session(&root.join("sessions"), clean_taint())).unwrap();
        assert!(matches!(
            store.record(&other, draft(), "m", &known()).unwrap(),
            Recorded::Written { .. }
        ));
        // A store that cannot be read cannot say "none on record".
        std::fs::remove_file(store.ledger()).unwrap();
        std::fs::create_dir_all(store.ledger()).unwrap();
        assert!(store.on_record(evidence.session_id()).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A judgment's goal crosses into the record only if a store that mints
    /// goal pointers holds it; one that does not resolve, or a word that
    /// was no pointer at all, leaves the judgment goal-less and is counted.
    #[test]
    fn a_judgments_goal_is_kept_only_when_a_store_holds_it() {
        let root = temp_root("goals");
        let evidence =
            SessionEvidence::read(&session(&root.join("sessions"), clean_taint())).unwrap();
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        let mut d = draft();
        d.judgments.push(Judgment {
            goal: Some(GoalRef::Task("t-ghost".into())),
            bearing: Bearing::Bad,
            because: vec![],
        });
        d.judgments.push(Judgment {
            goal: Some(GoalRef::Setpoint("inbox".into())),
            bearing: Bearing::Bad,
            because: vec![],
        });
        d.unreadable_goals = 1;
        store.record(&evidence, d, "m", &known()).unwrap();
        let got = &store.for_owner().unwrap().0[0];
        assert_eq!(
            got.judgments[0].goal,
            Some(GoalRef::Task("t-budget".into()))
        );
        assert_eq!(got.judgments[1].goal, None, "not on the board");
        assert_eq!(got.judgments[2].goal, None, "a setpoint never resolves");
        assert_eq!(got.goals_unresolved, 3, "two unresolved and one unreadable");
        assert_eq!(Summary::of(std::slice::from_ref(got)).goals_unresolved, 3);
        // Nothing read admits nothing.
        let none = SessionEvidence::read(&session(&root.join("sessions"), clean_taint())).unwrap();
        store
            .record(&none, draft(), "m", &crate::distill::KnownPointers::none())
            .unwrap();
        assert_eq!(store.for_owner().unwrap().0[1].judgments[0].goal, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The prediction's structural half (the owner's ruling, 2026-09-25):
    /// R16's six acts, stored beside the prose, loading leniently — a word a
    /// newer build wrote is `unknown`, a non-string does not cost the row,
    /// and a row from before the field has none.
    #[test]
    fn the_expected_act_is_a_closed_set_that_loads_leniently() {
        for a in ExpectedAct::ALL {
            assert_eq!(ExpectedAct::parse(a.wire()), Some(a));
            assert_eq!(crate::appraisal::enum_name(&a), a.wire());
        }
        assert_eq!(ExpectedAct::parse("No act"), Some(ExpectedAct::NoAct));
        assert_eq!(ExpectedAct::parse("unknown"), None, "not a producer's word");
        assert_eq!(ExpectedAct::parse("delighted"), None);

        let root = temp_root("act");
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        let evidence =
            SessionEvidence::read(&session(&root.join("sessions"), clean_taint())).unwrap();
        store.record(&evidence, draft(), "m", &known()).unwrap();
        let written = std::fs::read_to_string(store.ledger()).unwrap();
        assert!(
            written.contains(r#""expected_act":"released_unchanged""#),
            "{written}"
        );
        let raw = format!(
            "{}{}\n{}\n{}\n{}\n",
            written,
            r#"{"id":"apr-new","at":"2026-09-25T00:00:00Z","expected_act":"sulked"}"#,
            r#"{"id":"apr-num","at":"2026-09-25T00:00:00Z","expected_act":42}"#,
            r#"{"id":"apr-null","at":"2026-09-25T00:00:00Z","expected_act":null}"#,
            r#"{"id":"apr-old","at":"2026-09-25T00:00:00Z"}"#,
        );
        std::fs::write(store.ledger(), raw).unwrap();
        let (rows, skipped) = store.for_owner().unwrap();
        assert_eq!(skipped, 0, "no row is lost to the field");
        let acts: Vec<Option<ExpectedAct>> = rows.iter().map(|r| r.expected_act).collect();
        assert_eq!(
            acts,
            vec![
                Some(ExpectedAct::ReleasedUnchanged),
                Some(ExpectedAct::Unknown),
                Some(ExpectedAct::Unknown),
                None,
                None
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A fixture session whose run was matched toward `goal` (2c-1's
    /// `RunConfig::rules_goal`) — the key "same goal" is read off.
    fn session_toward(dir: &Path, goal: Option<crate::situation::GoalKey>) -> SessionEvidence {
        let path = session(dir, clean_taint());
        let s = Session {
            meta: Session::read(&path).unwrap().meta,
            path: path.clone(),
        };
        s.append(&Record::Config(RunConfig {
            tools: vec!["mail_search".into()],
            rules_workspace: Some(PathBuf::from("/project")),
            rules_surface: Some(SessionKind::Task),
            rules_goal: goal,
            ..Default::default()
        }))
        .unwrap();
        s.append(&Record::Taint(clean_taint().unwrap())).unwrap();
        let e = SessionEvidence::read(&path).unwrap();
        assert_eq!(e.origin(), Origin::Clean);
        e
    }

    /// I2 (2c-2): what a run is served is keyed like the appraiser's door,
    /// clean only, and on the tool set the run record will name — the
    /// registry after a front-end withheld a tool, not the one the block was
    /// matched against. A tainted appraisal of the very same situation is
    /// never served; another goal's never is; and a delegated task whose
    /// block saw `kg_task_update` still finds its past runs, which recorded
    /// the registry without it. Fails on a selection keyed on the build's
    /// registry, which served the task nothing.
    #[test]
    fn a_run_is_served_clean_appraisals_of_its_recorded_situation_and_goal() {
        use crate::situation::GoalKey;
        let toward = |g: &str, tools: &[&str]| {
            Situation::of_run(
                &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                Some(Path::new("/project")),
            )
            .on(Some(SessionKind::Task))
            .toward(Some(GoalKey::Named(g.parse().unwrap())))
        };
        let recorded = toward("task:t-budget", &["mail_search"]);
        let read = clean_read_of(vec![
            test_row("s-old", &recorded, true, "2026-09-20T00:00:00Z"),
            test_row("s-new", &recorded, true, "2026-09-22T00:00:00Z"),
            test_row("s-tainted", &recorded, false, "2026-09-23T00:00:00Z"),
            test_row(
                "s-other-goal",
                &toward("task:t-other", &["mail_search"]),
                true,
                "2026-09-24T00:00:00Z",
            ),
        ]);
        assert_eq!(read.withheld, 1, "the tainted row never becomes a Clean");
        // The block was matched with `kg_task_update` registered; the run
        // record, and so every past appraisal, names the registry without it.
        let built = toward("task:t-budget", &["kg_task_update", "mail_search"]);
        let mut past = PastAppraisals::select(read, &built);
        assert!(
            past.served().is_empty(),
            "keyed on the build's registry: nothing"
        );
        past.for_registry(&["mail_search".to_string()]);
        let got: Vec<&str> = past
            .served()
            .iter()
            .map(|c| c.session_id.as_str())
            .collect();
        assert_eq!(
            got,
            vec!["s-new", "s-old"],
            "newest first, clean, same goal"
        );
        assert_eq!(past.goal(), Some(&GoalRef::Task("t-budget".into())));
        assert!(past.unread_reason().is_none());
        let unread = PastAppraisals::unread("permission denied".into(), &built);
        assert!(unread.served().is_empty());
        assert_eq!(unread.unread_reason(), Some("permission denied"));
    }

    /// The past an appraiser is shown: clean appraisals of the same
    /// situation and goal key — newest first, at most `n`, never the
    /// session's own, never a tainted one (the type cannot hold it). The
    /// goal key is 2c-1's, compared exactly, so nothing widens: another
    /// goal, no goal, and a goal this build cannot name each match nothing
    /// but their own kind, and the unnameable not even that.
    #[test]
    fn past_appraisals_are_clean_only_and_of_the_same_situation_and_goal() {
        use crate::situation::GoalKey;
        let root = temp_root("past");
        let dir = root.join("sessions");
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();
        let mut same = Vec::new();
        for _ in 0..4 {
            let e = SessionEvidence::read(&session(&dir, clean_taint())).unwrap();
            store.record(&e, draft(), "m", &known()).unwrap();
            same.push(e.session_id().to_string());
        }
        let tainted = SessionEvidence::read(&session(&dir, tainted())).unwrap();
        store.record(&tainted, draft(), "m", &known()).unwrap();
        let other_goal =
            session_toward(&dir, Some(GoalKey::Named(GoalRef::Task("t-other".into()))));
        store.record(&other_goal, draft(), "m", &known()).unwrap();
        let no_goal = session_toward(&dir, None);
        store.record(&no_goal, draft(), "m", &known()).unwrap();
        let unread = session_toward(&dir, Some(GoalKey::Unread("quest:y".into())));
        store.record(&unread, draft(), "m", &known()).unwrap();

        let now = SessionEvidence::read(&session(&dir, clean_taint())).unwrap();
        let read = store.clean().unwrap();
        let past = read.same_situation_and_goal(&now, PAST_SHOWN);
        let got: Vec<&str> = past.iter().map(|c| c.session_id.as_str()).collect();
        let mut want: Vec<&str> = same.iter().map(String::as_str).collect();
        want.reverse();
        want.truncate(PAST_SHOWN);
        assert_eq!(got, want, "the three newest of the same situation and goal");
        for never in [&tainted, &other_goal, &no_goal, &unread] {
            assert!(!got.contains(&never.session_id()));
        }
        // A run that presented no goal is shown only the goal-less record —
        // never a goal-scoped one.
        let goalless = read.same_situation_and_goal(&session_toward(&dir, None), 10);
        let ids: Vec<&str> = goalless.iter().map(|c| c.session_id.as_str()).collect();
        assert_eq!(ids, vec![no_goal.session_id()]);
        // A goal this build cannot name matches nothing, itself included.
        let unnamed = session_toward(&dir, Some(GoalKey::Unread("quest:y".into())));
        assert!(read.same_situation_and_goal(&unnamed, 10).is_empty());
        // Never the session's own.
        let first = SessionEvidence::read(&Session::find(&dir, &same[0]).unwrap()).unwrap();
        let theirs = read.same_situation_and_goal(&first, 10);
        assert_eq!(theirs.len(), 3);
        assert!(theirs.iter().all(|c| c.session_id != same[0]));
        let _ = std::fs::remove_dir_all(&root);
    }

    // --- the appraisal's own prediction, scored (row 2b-2) ---

    /// A model-authored draft staged in `session`, then set to `status` at
    /// `resolved_at` — the shape the outbox store keeps.
    fn draft_of(
        root: &Path,
        session: &str,
        status: &str,
        resolved_at: Option<DateTime<Utc>>,
    ) -> crate::outbox::OutboxItem {
        let store =
            crate::outbox::OutboxStore::open(root.join(format!("outbox-{}", uuid::Uuid::new_v4())))
                .unwrap();
        let mut item = store
            .stage(
                "mail_send",
                crate::outbox::OutboxKind::Message,
                json!({"to": "idris.vale@example.org", "body": "The review is Thursday."}),
                Taint::default(),
                crate::outbox::Provenance {
                    anticipation: None,
                    filled_defaults: Vec::new(),
                    session_id: Some(session.to_string()),
                    workspace: None,
                    call_id: None,
                },
            )
            .unwrap();
        item.status = status.to_string();
        item.resolved_at = resolved_at.map(|t| t.to_rfc3339());
        item
    }

    fn acts<'a>(drafts: &'a [crate::outbox::OutboxItem]) -> OwnerActs<'a> {
        OwnerActs {
            drafts,
            ..OwnerActs::default()
        }
    }

    /// R37's boundary: "no act" becomes the act that happened only once the
    /// output's store patience has elapsed from the session's end — the
    /// doctor's constant, or the charter line watching the outbox. An act
    /// before the window closes is the act; one after it is not.
    #[test]
    fn no_act_resolves_only_after_the_outputs_patience_from_the_sessions_end() {
        let root = temp_root("window");
        let end: DateTime<Utc> = "2026-09-20T12:00:00Z".parse().unwrap();
        let second = chrono::Duration::seconds(1);
        let pending = [draft_of(&root, "s-1", "pending", None)];
        let hours = |h: i64| chrono::Duration::hours(h);

        // The doctor's constant for the outbox, 48h.
        let at = |now| observe("s-1", None, end, &acts(&pending), now);
        assert_eq!(
            at(end + hours(48) - second),
            ObservedAct::Pending {
                closes_at: end + hours(48)
            },
            "just before the window closes: still waiting"
        );
        assert_eq!(
            at(end + hours(48) + second),
            ObservedAct::NoAct {
                window_closed_at: end + hours(48)
            },
            "just after: no act is the act"
        );

        // The owner's own line on the outbox moves the window.
        let charter = crate::charter::Charter::parse(
            "[[line]]\nid = \"replies\"\ntext = \"Answer the people waiting on me.\"\n\
             [line.sensor]\nkind = \"outbox_age\"\nsetpoint = \"24h\"\n",
        )
        .unwrap();
        let owned = OwnerActs {
            charter: Some(&charter),
            ..acts(&pending)
        };
        assert!(matches!(
            observe("s-1", None, end, &owned, end + hours(24) - second),
            ObservedAct::Pending { .. }
        ));
        assert!(matches!(
            observe("s-1", None, end, &owned, end + hours(24) + second),
            ObservedAct::NoAct { .. }
        ));

        // An act inside the window is the act, whenever it is read.
        let rejected = [draft_of(&root, "s-1", "rejected", Some(end + hours(47)))];
        assert_eq!(
            observe("s-1", None, end, &acts(&rejected), end + hours(100)),
            ObservedAct::Act {
                act: ExpectedAct::Rejected,
                at: end + hours(47)
            }
        );
        // One after it is not: the window closed on no act.
        let late = [draft_of(&root, "s-1", "sent", Some(end + hours(49)))];
        assert_eq!(
            observe("s-1", None, end, &acts(&late), end + hours(100)),
            ObservedAct::NoAct {
                window_closed_at: end + hours(48)
            }
        );
        // Another session's draft is not this session's act; with no draft
        // of its own the output has no store, and the constant applies.
        assert!(matches!(
            observe("s-2", None, end, &acts(&rejected), end + hours(47)),
            ObservedAct::Pending { .. }
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An owner closure of `task` naming session `session`, at `at`.
    fn closure(task: &str, session: &str, at: DateTime<Utc>) -> crate::closure::Transition {
        crate::closure::Transition {
            at,
            ..serde_json::from_value(json!({
                "id": format!("cl-{task}-{session}"), "task": task, "from": "next",
                "to": "done", "move": "close", "actor": "owner", "surface": "cli",
                "sessions": [session], "at": "2026-09-20T12:00:00Z"
            }))
            .unwrap()
        }
    }

    /// R37 refined by the owner (2026-09-25): a task output's no-act window
    /// runs from the session's end to the task's `due_at` on the board. A
    /// closure by the due date is the act; one after it is not. An undated
    /// task keeps the constant; a date already past at the session's end
    /// falls back to the constant; an unreadable board or an unparseable
    /// date is unknown, never the constant; a caller that read no board
    /// defers to one that does.
    #[test]
    fn a_task_outputs_window_runs_to_its_due_date() {
        let end: DateTime<Utc> = "2026-09-20T12:00:00Z".parse().unwrap();
        let second = chrono::Duration::seconds(1);
        let hours = |h: i64| chrono::Duration::hours(h);
        let task = GoalRef::Task("task-1".into());
        let board = |due: serde_json::Value| json!({"items": [{"id": "task-1", "status": "done", "due_at": due}]});
        let look = |closures: &[crate::closure::Transition],
                    board: BoardRead<'_>,
                    zone: Option<chrono_tz::Tz>,
                    now: DateTime<Utc>| {
            observe(
                "s-3",
                Some(&task),
                end,
                &OwnerActs {
                    closures,
                    board,
                    zone,
                    ..OwnerActs::default()
                },
                now,
            )
        };

        // Due at an instant four days out: the window closes there, not at
        // the 48h constant.
        let due = end + hours(96);
        let dated = board(json!(due.to_rfc3339()));
        assert_eq!(
            look(&[], BoardRead::Read(&dated), None, end + hours(49)),
            ObservedAct::Pending { closes_at: due },
            "past the constant, still inside the due date"
        );
        assert_eq!(
            look(
                &[closure("task-1", "s-3", due - second)],
                BoardRead::Read(&dated),
                None,
                due + hours(1)
            ),
            ObservedAct::Act {
                act: ExpectedAct::Closed,
                at: due - second
            },
            "a closure just before the due date is the act"
        );
        assert_eq!(
            look(
                &[closure("task-1", "s-3", due + second)],
                BoardRead::Read(&dated),
                None,
                due + hours(1)
            ),
            ObservedAct::NoAct {
                window_closed_at: due
            },
            "a closure just after it is not"
        );

        // A due *date* is the end of that day where the owner is.
        let day = board(json!("2026-09-22"));
        let ny: chrono_tz::Tz = "America/New_York".parse().unwrap();
        let end_of_day_ny: DateTime<Utc> = "2026-09-23T04:00:00Z".parse().unwrap();
        assert_eq!(
            look(&[], BoardRead::Read(&day), Some(ny), end + hours(1)),
            ObservedAct::Pending {
                closes_at: end_of_day_ny
            }
        );
        let end_of_day_utc: DateTime<Utc> = "2026-09-23T00:00:00Z".parse().unwrap();
        assert_eq!(
            look(&[], BoardRead::Read(&day), None, end + hours(1)),
            ObservedAct::Pending {
                closes_at: end_of_day_utc
            }
        );

        // An undated task keeps the constant: a closure at 47h is the act,
        // one at 49h is not.
        let undated = board(serde_json::Value::Null);
        assert_eq!(
            look(
                &[closure("task-1", "s-3", end + hours(47))],
                BoardRead::Read(&undated),
                None,
                end + hours(100)
            ),
            ObservedAct::Act {
                act: ExpectedAct::Closed,
                at: end + hours(47)
            }
        );
        assert!(matches!(
            look(
                &[closure("task-1", "s-3", end + hours(49))],
                BoardRead::Read(&undated),
                None,
                end + hours(100)
            ),
            ObservedAct::NoAct { .. }
        ));

        // A due date already past when the session ended: the constant.
        let overdue = board(json!((end - hours(24)).to_rfc3339()));
        assert_eq!(
            look(&[], BoardRead::Read(&overdue), None, end + hours(1)),
            ObservedAct::Pending {
                closes_at: end + hours(NO_STORE_PATIENCE_HOURS)
            }
        );

        // Unknown, never the constant: an unreadable board, a date that will
        // not parse, a board with no row for the task.
        let long_after = end + hours(24 * 30);
        let torn = board(json!("next Tuesday"));
        let elsewhere = json!({"items": [{"id": "task-9"}]});
        for (what, b) in [
            ("an unreadable board", BoardRead::Unreadable),
            ("an unparseable due date", BoardRead::Read(&torn)),
            (
                "a board with no row for the task",
                BoardRead::Read(&elsewhere),
            ),
        ] {
            assert!(
                matches!(look(&[], b, None, long_after), ObservedAct::Unknown { .. }),
                "{what}"
            );
        }
        // A reader that read no board defers — neither unknown nor windowed.
        assert_eq!(
            look(&[], BoardRead::NotRead, None, long_after),
            ObservedAct::NeedsBoard
        );

        // The task is found through a closure naming the session when the
        // session carries no task anchor.
        let via_closure = observe(
            "s-4",
            None,
            end,
            &OwnerActs {
                closures: &[closure("task-1", "s-4", due - second)],
                board: BoardRead::Read(&dated),
                ..OwnerActs::default()
            },
            due + hours(1),
        );
        assert!(matches!(
            via_closure,
            ObservedAct::Act {
                act: ExpectedAct::Closed,
                ..
            }
        ));
    }

    /// An act store that could not be read, or a patience that could not
    /// be, is unknown — never "no act", however long ago the session ended.
    #[test]
    fn an_unreadable_store_or_patience_is_unknown_never_no_act() {
        let root = temp_root("unknown");
        let end: DateTime<Utc> = "2026-09-01T12:00:00Z".parse().unwrap();
        let long_after = end + chrono::Duration::days(30);
        let pending = [draft_of(&root, "s-1", "pending", None)];
        for (what, a) in [
            (
                "outbox",
                OwnerActs {
                    outbox_unreadable: true,
                    ..acts(&pending)
                },
            ),
            (
                "closures",
                OwnerActs {
                    closures_unreadable: true,
                    ..acts(&pending)
                },
            ),
            (
                "workflows",
                OwnerActs {
                    workflows_unreadable: true,
                    ..acts(&pending)
                },
            ),
            (
                "charter, so the outbox's patience",
                OwnerActs {
                    charter_unreadable: true,
                    ..acts(&pending)
                },
            ),
        ] {
            assert!(
                matches!(
                    observe("s-1", None, end, &a, long_after),
                    ObservedAct::Unknown { .. }
                ),
                "{what}"
            );
        }
        // A closure naming this session, by the owner, whose move a newer
        // build wrote: an act seen and not read — unknown, never no act.
        let future: crate::closure::Transition = serde_json::from_value(json!({
            "id": "cl-9", "task": "task-1", "to": "archived", "move": "archive",
            "actor": "owner", "surface": "cli", "sessions": ["s-1"],
            "at": "2026-09-02T12:00:00Z"
        }))
        .unwrap();
        assert!(matches!(
            observe(
                "s-1",
                None,
                end,
                &OwnerActs {
                    closures: std::slice::from_ref(&future),
                    ..OwnerActs::default()
                },
                long_after
            ),
            ObservedAct::Unknown { .. }
        ));
        // A row naming no session has no output to read.
        assert!(matches!(
            observe("", None, end, &OwnerActs::default(), long_after),
            ObservedAct::Unknown { .. }
        ));
        // A resolved draft with no readable time cannot be placed in the
        // window either.
        let untimed = [draft_of(&root, "s-1", "sent", None)];
        assert!(matches!(
            observe("s-1", None, end, &acts(&untimed), long_after),
            ObservedAct::Unknown { .. }
        ));
        // With no draft there is no store, so an unreadable charter does
        // not stand between the constant and the answer.
        assert!(matches!(
            observe(
                "s-1",
                None,
                end,
                &OwnerActs {
                    charter_unreadable: true,
                    ..OwnerActs::default()
                },
                long_after
            ),
            ObservedAct::NoAct { .. }
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Row 2b-2's acceptance: a pair of sessions scores a hit and a miss on
    /// `expected_act` against the recorded act, the miss is recorded as a
    /// surprise, each score is written once, and coverage has no rate over
    /// nothing. No model decides a score (R27).
    #[test]
    fn a_hit_and_a_surprise_are_scored_once_and_coverage_has_no_rate_over_nothing() {
        let root = temp_root("scores");
        let dir = root.join("sessions");
        let store = AppraisalStore::open(root.join("appraisals")).unwrap();

        // Nothing on record: coverage, no rate.
        let empty = store
            .score_summary(&OwnerActs::default(), Utc::now())
            .unwrap();
        assert_eq!((empty.appraisals, empty.scored), (0, 0));
        assert_eq!(empty.hit_rate, None);

        // Two sessions: one expected to draw no act, one expected to be
        // released unchanged — and the owner rejects its draft.
        let quiet = SessionEvidence::read(&session(&dir, clean_taint())).unwrap();
        let busy = SessionEvidence::read(&session(&dir, tainted())).unwrap();
        store
            .record(
                &quiet,
                Draft {
                    expected_act: Some(ExpectedAct::NoAct),
                    ..draft()
                },
                "m",
                &known(),
            )
            .unwrap();
        store
            .record(
                &busy,
                Draft {
                    expected_act: Some(ExpectedAct::ReleasedUnchanged),
                    ..draft()
                },
                "m",
                &known(),
            )
            .unwrap();
        // A third appraisal names no act: nothing to score.
        let silent = SessionEvidence::read(&session(&dir, clean_taint())).unwrap();
        store
            .record(
                &silent,
                Draft {
                    expected_act: None,
                    ..draft()
                },
                "m",
                &known(),
            )
            .unwrap();
        let rows = store.for_owner().unwrap().0;
        assert!(
            rows.iter().all(|r| r.session_ended_at.is_some()),
            "the session's end is recorded with the appraisal"
        );
        let rejected_at = Utc::now() + chrono::Duration::hours(1);
        let drafts = [draft_of(
            &root,
            busy.session_id(),
            "rejected",
            Some(rejected_at),
        )];
        // Both sessions are anchored to `t-budget`; the board has it with no
        // due date, so the quiet one's window is the constant (R37 refined).
        let tasks = json!({"items": [{"id": "t-budget", "status": "next"}]});
        let owner = OwnerActs {
            board: BoardRead::Read(&tasks),
            ..acts(&drafts)
        };

        // Read-only before any pass: the rejection has already happened, so
        // that one is resolved-but-unwritten, not waiting.
        let before = store
            .score_summary(&owner, Utc::now() + chrono::Duration::hours(2))
            .unwrap();
        assert_eq!(
            (before.scored, before.resolved_unwritten, before.pending),
            (0, 1, 1)
        );

        // Before the quiet session's window closes: one scored, one waiting.
        let early = store
            .score_due(&owner, Utc::now() + chrono::Duration::hours(2))
            .unwrap();
        assert_eq!(
            (early.with_expectation, early.scored, early.pending),
            (2, 1, 1)
        );
        assert_eq!((early.hits, early.surprises), (0, 1));
        assert_eq!(early.hit_rate, Some(0.0));

        // After it: the no-act prediction is a hit.
        let later = Utc::now() + chrono::Duration::hours(72);
        let done = store.score_due(&owner, later).unwrap();
        assert_eq!(
            (done.scored, done.hits, done.surprises, done.pending),
            (2, 1, 1, 0)
        );
        assert_eq!(done.hit_rate, Some(0.5));
        assert_eq!(
            done.clean_surprises, 0,
            "the surprise is on the tainted appraisal"
        );

        let (scores, skipped) = store.scores().unwrap();
        assert_eq!((scores.len(), skipped), (2, 0));
        let miss = scores
            .iter()
            .find(|s| s.session_id == busy.session_id())
            .unwrap();
        assert_eq!(
            (
                miss.expected,
                miss.actual,
                miss.hit,
                miss.surprise,
                miss.clean
            ),
            (
                ExpectedAct::ReleasedUnchanged,
                ExpectedAct::Rejected,
                false,
                true,
                false
            )
        );
        assert!(miss.acted_at.is_some());
        let hit = scores
            .iter()
            .find(|s| s.session_id == quiet.session_id())
            .unwrap();
        assert_eq!(
            (hit.actual, hit.hit, hit.surprise, hit.clean),
            (ExpectedAct::NoAct, true, false, true)
        );
        assert!(hit.window_closed_at.is_some());

        // Scored once: a second pass writes nothing more.
        store.score_due(&owner, later).unwrap();
        assert_eq!(store.scores().unwrap().0.len(), 2);
        let busy_row = rows
            .iter()
            .find(|r| r.session_id == busy.session_id())
            .unwrap();
        assert_eq!(
            store.score(busy_row, &owner, later).unwrap(),
            Scored::AlreadyScored
        );

        // An unreadable act store scores nothing and says unknown.
        let fresh = AppraisalStore::open(root.join("appraisals-2")).unwrap();
        fresh
            .record(
                &quiet,
                Draft {
                    expected_act: Some(ExpectedAct::NoAct),
                    ..draft()
                },
                "m",
                &known(),
            )
            .unwrap();
        let blind = OwnerActs {
            outbox_unreadable: true,
            ..OwnerActs::default()
        };
        let s = fresh.score_due(&blind, later).unwrap();
        assert_eq!((s.scored, s.unknown, s.hit_rate), (0, 1, None));
        assert!(fresh.scores().unwrap().0.is_empty());

        // An appraisal line that cannot be read is counted, never silently
        // out of the denominator.
        let mut ledger = std::fs::read_to_string(fresh.ledger()).unwrap();
        ledger.push_str("{\"id\":\"apr-torn\",\"expected_act\n");
        std::fs::write(fresh.ledger(), ledger).unwrap();
        let s = fresh.score_summary(&blind, later).unwrap();
        assert_eq!(s.appraisals_unreadable, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The score ledger is a wire format: an act word this build cannot
    /// read loads as `unknown`, and a torn line costs only itself.
    #[test]
    fn the_score_ledger_loads_leniently() {
        let root = temp_root("score-wire");
        let store = AppraisalStore::open(&root).unwrap();
        std::fs::write(
            store.scores_ledger(),
            "{\"id\":\"scr-1\",\"scored_at\":\"2026-09-25T00:00:00Z\",\"appraisal_id\":\"apr-1\",\
             \"expected\":\"sulked\",\"actual\":\"no_act\",\"hit\":false,\"surprise\":true}\n\
             {\"id\":\"scr-torn\",\"scored_at\n",
        )
        .unwrap();
        let (scores, skipped) = store.scores().unwrap();
        assert_eq!((scores.len(), skipped), (1, 1));
        assert_eq!(scores[0].expected, ExpectedAct::Unknown);
        assert_eq!(scores[0].actual, ExpectedAct::NoAct);
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
