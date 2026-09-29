//! The harness's forecast of the owner's act on a draft, made when the
//! draft is staged — the owner's ruling (a) of 2026-09-29, v1 (X3,
//! unparked).
//!
//! **No model chooses the act.** v1 is a base rate: the act the owner most
//! often took on earlier model-authored message drafts staged through the
//! same tool, in the same armed state (private and untrusted both in
//! context), counting only acts the owner's stamp proves were the owner's
//! ([`crate::outbox::OutboxItem::owners_unchanged_release`], `owners_edit`,
//! a reject stamped [`crate::closure::Actor::Owner`]) and drafts that sat
//! past the outbox's patience untouched (`no_act`, R37 carried to items).
//! A model forecaster comes later, as an arm measured against this one on
//! the same drafts.
//!
//! **Sealed.** A forecast is written to `<outbox>/forecasts/forecasts.jsonl`
//! beside the items and nowhere else: no per-draft surface, no staging
//! result, no prompt reads it — a forecast of the owner's approval shown to
//! the reviewer or the acting model is a way to steer the verdict. It is
//! read back only by [`summarize`], for the `sessions appraise` readout.
//!
//! **Readout only.** A miss is counted, never fed to replay priority, until
//! the forecasts are calibrated (the owner's ruling of 2026-09-29).
//!
//! **Pre-registered by construction.** The forecast is written at staging,
//! from history resolved before it, so nothing learned from the draft's own
//! outcome can reach it.
//!
//! **Only a run's drafts.** A draft is forecast, and counts as history, only
//! when it carries the session that staged it ([`forecasts`]). That keeps
//! the owner's own typed text (`mecha mail send`) out. It also leaves out
//! a route that stamps no session: `mecha batch` stamps none, so its drafts
//! are neither forecast nor counted as unforecast. That is by design, not a
//! lost write.

use crate::appraisal_store::ExpectedAct;
use crate::closure::Actor;
use crate::outbox::{Author, OutboxItem, OutboxKind};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The acts a draft forecast may name: the outbox's three verdicts, and
/// none within the patience.
pub const DRAFT_ACTS: [ExpectedAct; 4] = [
    ExpectedAct::ReleasedUnchanged,
    ExpectedAct::Edited,
    ExpectedAct::Rejected,
    ExpectedAct::NoAct,
];

/// Who made the forecast. A closed set on an append-only ledger: a word a
/// newer build wrote loads as `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The owner's most frequent stamped act on similar drafts.
    #[default]
    BaseRate,
    #[serde(other)]
    Unknown,
}

/// One forecast, written once at staging.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// When it was made: the draft's staging.
    pub at: DateTime<Utc>,
    /// The key "similar" is judged by.
    pub tool: String,
    pub armed: bool,
    /// The act forecast; `None` where no stamped act on a similar draft
    /// exists yet — no basis, which is never a guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<ExpectedAct>,
    /// How many of the owner's acts on similar drafts it rests on.
    #[serde(default)]
    pub basis: usize,
    /// No basis because the history or the window could not be read — not
    /// because none has accumulated. Kept apart on the record, since the two
    /// are indistinguishable afterwards otherwise (review of #401).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub basis_unreadable: bool,
    #[serde(default)]
    pub source: Source,
}

/// Where the ledger lives for an outbox rooted at `outbox_root` — in a
/// subdirectory, out of the item walk.
pub fn ledger(outbox_root: &Path) -> PathBuf {
    outbox_root.join("forecasts").join("forecasts.jsonl")
}

/// Whether an item is a draft this forecasts: a message a run drafted,
/// with a body. `Author::Model` alone is not enough — `mecha mail send`
/// stages the owner's own typed text through the same `stage`, as
/// `author: model` — so the draft must also carry the session that staged
/// it, which a run's route stamps and the owner's CLI does not (review of
/// #401). One predicate for what is forecast, what counts as history, and
/// what counts as unforecast.
pub fn forecasts(item: &OutboxItem) -> bool {
    item.kind == OutboxKind::Message
        && item.author() == Author::Model
        && item
            .session_id
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        && crate::outbox::DraftView::of(&item.args).body.is_some()
}

fn armed(item: &OutboxItem) -> bool {
    item.taint.private && item.taint.untrusted
}

fn at(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// What happened to a staged draft, as far as `now` and the stamps can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// The owner's act, stamped as theirs, inside the patience window.
    Act(ExpectedAct),
    /// The window closed with no act — including an act that came after it.
    NoAct,
    /// Still inside the window, untouched.
    Pending,
    /// Resolved by someone the stamps do not show to be the owner, or with
    /// a time or status this build cannot read: not the owner's act, and
    /// not "no act" either.
    Unknown,
}

/// The owner's act on `item` within `patience` of its staging, as of `now`.
pub fn observe(item: &OutboxItem, patience: chrono::Duration, now: DateTime<Utc>) -> Observed {
    let Some(staged) = at(&item.created_at) else {
        return Observed::Unknown;
    };
    let closes = staged + patience;
    // A release whose delivery failed stays `pending` on purpose
    // (`OutboxStore::record_error`): the owner acted and the wire did not.
    // Not the owner's act by the stamps, and never "no act" (review of #401).
    if item.status == "pending" && (!item.delivery_attempts.is_empty() || item.error.is_some()) {
        return Observed::Unknown;
    }
    if item.status == "pending" {
        return if now >= closes {
            Observed::NoAct
        } else {
            Observed::Pending
        };
    }
    let Some(resolved) = item.resolved_at.as_deref().and_then(at) else {
        return Observed::Unknown;
    };
    if resolved > closes {
        return Observed::NoAct;
    }
    match item.status.as_str() {
        "sent" if item.owners_edit() => Observed::Act(ExpectedAct::Edited),
        "sent" if item.owners_unchanged_release() => Observed::Act(ExpectedAct::ReleasedUnchanged),
        "rejected" if item.resolved_by == Some(Actor::Owner) => {
            Observed::Act(ExpectedAct::Rejected)
        }
        _ => Observed::Unknown,
    }
}

/// The base rate for a draft staged at `now` through `tool` in `armed`
/// state: the owner's most frequent act on earlier similar drafts whose
/// outcome was settled by `now`, with how many it rests on. Ties go to the
/// earlier act in [`DRAFT_ACTS`]; no history is `(None, 0)`.
pub fn base_rate(
    tool: &str,
    is_armed: bool,
    history: &[OutboxItem],
    patience: chrono::Duration,
    now: DateTime<Utc>,
) -> (Option<ExpectedAct>, usize) {
    let mut counts = [0usize; DRAFT_ACTS.len()];
    for item in history {
        if !forecasts(item) || item.tool != tool || armed(item) != is_armed {
            continue;
        }
        if at(&item.created_at).is_none_or(|t| t >= now) {
            continue;
        }
        let act = match observe(item, patience, now) {
            Observed::Act(a) => a,
            Observed::NoAct => ExpectedAct::NoAct,
            Observed::Pending | Observed::Unknown => continue,
        };
        if let Some(i) = DRAFT_ACTS.iter().position(|a| *a == act) {
            counts[i] += 1;
        }
    }
    let basis: usize = counts.iter().sum();
    if basis == 0 {
        return (None, 0);
    }
    let best = counts
        .iter()
        .enumerate()
        .max_by(|(i, a), (j, b)| a.cmp(b).then(j.cmp(i)))
        .map(|(i, _)| DRAFT_ACTS[i]);
    (best, basis)
}

/// Where a store that forecasts takes its patience window from. A store
/// forecasts only when told to ([`crate::outbox::OutboxStore::with_forecasts`]):
/// the agent's route turns it on with [`Window::Charter`], and a test with
/// [`Window::Fixed`], so no test reads the machine's charter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// The charter line watching the outbox, else the doctor's constant.
    Charter,
    Fixed(chrono::Duration),
}

impl Window {
    pub fn patience(self) -> Option<chrono::Duration> {
        match self {
            Window::Charter => outbox_patience(),
            Window::Fixed(d) => Some(d),
        }
    }
}

/// The outbox's patience, as the doctor and the appraisal scorer read it:
/// the charter line watching the outbox, else the doctor's constant.
/// `None` when the charter cannot be read — the window is unknown.
pub fn outbox_patience() -> Option<chrono::Duration> {
    let (charter, unreadable) = crate::appraisal::load_charter();
    if unreadable {
        return None;
    }
    crate::doctor::Patience::for_store(charter.as_ref(), crate::charter::SensorKind::OutboxAge)
        .map(|p| p.after)
}

/// Forecast a freshly staged `item` from `history` and append it to the
/// ledger. A draft this does not forecast writes nothing. An unknown window
/// (the charter unreadable) is a forecast with no basis, never a guess.
pub fn record(
    outbox_root: &Path,
    item: &OutboxItem,
    history: Option<&[OutboxItem]>,
    patience: Option<chrono::Duration>,
) -> Result<Option<Forecast>> {
    if !forecasts(item) {
        return Ok(None);
    }
    let now = at(&item.created_at).unwrap_or_else(Utc::now);
    let (expected, basis, basis_unreadable) = match (history, patience) {
        (Some(h), Some(p)) => {
            let (e, b) = base_rate(&item.tool, armed(item), h, p, now);
            (e, b, false)
        }
        _ => (None, 0, true),
    };
    let f = Forecast {
        item_id: item.id.clone(),
        session_id: item.session_id.clone(),
        at: now,
        tool: item.tool.clone(),
        armed: armed(item),
        expected,
        basis,
        basis_unreadable,
        source: Source::BaseRate,
    };
    let path = ledger(outbox_root);
    let dir = path.parent().expect("the ledger has a directory");
    crate::create_private_dir(dir).with_context(|| format!("creating {}", dir.display()))?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    let mut line = serde_json::to_string(&f)?;
    line.push('\n');
    file.write_all(line.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(Some(f))
}

/// Every forecast on record, and how many lines could not be read. A
/// missing ledger is none.
pub fn load(outbox_root: &Path) -> Result<(Vec<Forecast>, usize)> {
    let path = ledger(outbox_root);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(f) => out.push(f),
            Err(_) => skipped += 1,
        }
    }
    Ok((out, skipped))
}

/// The forecasts, scored against what happened — read-only, readout only.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    pub forecasts: usize,
    /// Made with no stamped history on a similar draft: nothing to score.
    pub no_basis: usize,
    /// Of those, made when the history or the window could not be read —
    /// a finding, not "none yet".
    pub basis_unreadable: usize,
    pub scored: usize,
    pub hits: usize,
    pub surprises: usize,
    /// Inside the window, untouched.
    pub pending: usize,
    /// Resolved by someone the stamps do not show to be the owner, or the
    /// draft is gone or unreadable: not scored, and not "no act".
    pub unknown: usize,
    /// Forecasts left unscored because the outbox's patience window could
    /// not be read now (the charter): a finding about the charter, never
    /// "not the owner's by the stamps" (review of #401).
    pub window_unreadable: usize,
    /// The outbox could not be read whole, so `drafts_unread` forecasts
    /// could not be scored and `unforecast` was not checked (review of
    /// #401): a read failure, said as one.
    pub outbox_unreadable: bool,
    /// Forecasts whose draft the partial outbox read did not see.
    pub drafts_unread: usize,
    /// Drafts this should have forecast, staged since the first forecast,
    /// with none on record — a write that failed, said rather than hidden.
    pub unforecast: usize,
    /// `hits / scored`; `None` over no scores.
    pub hit_rate: Option<f64>,
    /// Ledger lines that could not be read.
    pub skipped: usize,
}

/// Score every forecast against its draft. `patience` `None` — the charter
/// unreadable — scores nothing and says so as unknown.
pub fn summarize(
    made: &[Forecast],
    skipped: usize,
    items: &[OutboxItem],
    items_complete: bool,
    patience: Option<chrono::Duration>,
    now: DateTime<Utc>,
) -> Summary {
    let mut s = Summary {
        forecasts: made.len(),
        skipped,
        outbox_unreadable: !items_complete,
        ..Summary::default()
    };
    for f in made {
        let expected = match f.expected {
            None => {
                s.no_basis += 1;
                if f.basis_unreadable {
                    s.basis_unreadable += 1;
                }
                continue;
            }
            // An act word a newer build wrote: a basis existed, this build
            // cannot name it — unknown, never "no basis" (review of #401).
            Some(ExpectedAct::Unknown) => {
                s.unknown += 1;
                continue;
            }
            Some(a) => a,
        };
        let Some(p) = patience else {
            s.window_unreadable += 1;
            continue;
        };
        let observed = match items.iter().find(|i| i.id == f.item_id) {
            Some(item) => observe(item, p, now),
            // Not seen by a partial read: unread, never "gone".
            None if !items_complete => {
                s.drafts_unread += 1;
                continue;
            }
            None => Observed::Unknown,
        };
        let actual = match observed {
            Observed::Act(a) => a,
            Observed::NoAct => ExpectedAct::NoAct,
            Observed::Pending => {
                s.pending += 1;
                continue;
            }
            Observed::Unknown => {
                s.unknown += 1;
                continue;
            }
        };
        s.scored += 1;
        if actual == expected {
            s.hits += 1;
        } else {
            s.surprises += 1;
        }
    }
    // Coverage is checked only over a whole read.
    if let Some(first) = made.iter().map(|f| f.at).min().filter(|_| items_complete) {
        s.unforecast = items
            .iter()
            .filter(|i| forecasts(i) && at(&i.created_at).is_some_and(|t| t >= first))
            .filter(|i| !made.iter().any(|f| f.item_id == i.id))
            .count();
    }
    s.hit_rate = (s.scored > 0).then(|| s.hits as f64 / s.scored as f64);
    s
}

#[cfg(test)]
mod tests;
