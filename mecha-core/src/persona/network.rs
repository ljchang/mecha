//! Her network: the people in her life and how she stands with each
//! (`docs/PERSONA-LIFE-DESIGN.md` §6, ruled PL5).
//!
//! Two tables in `world.db`. **`people`** holds who someone is: a name,
//! aliases, their role in her life, a line about them, an optional library
//! character, and where the row came from. **`edge_events`** is an
//! append-only log of what happened between her and someone, or between two
//! people in her world: met, formed, strengthened, conflict, reconciled,
//! paused, ended, renewed. Current state is the fold of an edge's events
//! ([`fold`]), never a row rewritten in place, so history, dates and "how
//! did this change" come free, and `forget` can drop what one chat
//! contributed and fold again (§5, PL22).
//!
//! A date keeps the precision it was given (day, month, season, year or
//! none): "friends since school" is never forced into a false day, as
//! `capture.rs` detects *when* and never resolves it.
//!
//! **The owner writes this step's people** (§6.2): an owner row never
//! decays (PL7) and the simulation, when it comes, may add events to it but
//! never rewrite who someone is. Edits go through the CLI and the People
//! tab; `network.md` beside the store is rendered from it after every
//! write, for reading anywhere, and never parsed back (§6.6).
//!
//! Every closed set here is a wire format: a value this binary does not
//! know loads as an unknown that is never clean, and a fold skips an event
//! it cannot read rather than guessing what it said.

use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use super::memory::{now as stamp, uid};
use super::world::{Source, World};

/// The rendered network, beside `world.db`.
pub const NETWORK_MD: &str = "network.md";
/// A name, an alias, a role: a few words.
pub const MAX_NAME: usize = 80;
/// A tie's free label: "older sister", "lab partner".
pub const MAX_LABEL: usize = 40;
/// A line about someone.
pub const MAX_DESCRIPTION: usize = 400;
/// What an event was about: how they met, what the conflict was.
pub const MAX_CONTEXT: usize = 200;
pub const MAX_ALIASES: usize = 16;
/// Her network tops out near Dunbar's 150; this leaves room and stops a
/// runaway writer.
pub const MAX_PEOPLE: usize = 400;

/// What kind of tie an edge is, with a free label beside it (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tie {
    Kin,
    Romantic,
    Ex,
    Friend,
    Colleague,
    Mentor,
    Neighbour,
    Acquaintance,
    Rival,
    #[serde(other)]
    Unknown,
}

impl Tie {
    pub const ALL: [Tie; 9] = [
        Tie::Kin,
        Tie::Romantic,
        Tie::Ex,
        Tie::Friend,
        Tie::Colleague,
        Tie::Mentor,
        Tie::Neighbour,
        Tie::Acquaintance,
        Tie::Rival,
    ];
}

/// What happened on an edge (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Met,
    Formed,
    Strengthened,
    Conflict,
    Reconciled,
    Paused,
    Ended,
    Renewed,
    #[serde(other)]
    Unknown,
}

impl EventKind {
    pub const ALL: [EventKind; 8] = [
        EventKind::Met,
        EventKind::Formed,
        EventKind::Strengthened,
        EventKind::Conflict,
        EventKind::Reconciled,
        EventKind::Paused,
        EventKind::Ended,
        EventKind::Renewed,
    ];
}

/// How close: a Dunbar layer, innermost first (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    /// The closest five.
    Five,
    Fifteen,
    Fifty,
    OneFifty,
}

impl Layer {
    pub const ALL: [Layer; 4] = [Layer::Five, Layer::Fifteen, Layer::Fifty, Layer::OneFifty];

    /// The layer's size, as it is named and stored.
    pub fn size(self) -> u32 {
        match self {
            Layer::Five => 5,
            Layer::Fifteen => 15,
            Layer::Fifty => 50,
            Layer::OneFifty => 150,
        }
    }

    pub fn of_size(n: u32) -> Option<Layer> {
        Layer::ALL.into_iter().find(|l| l.size() == n)
    }

    /// One ring further in; the closest stays.
    fn inward(self) -> Layer {
        match self {
            Layer::Five | Layer::Fifteen => Layer::Five,
            Layer::Fifty => Layer::Fifteen,
            Layer::OneFifty => Layer::Fifty,
        }
    }
}

/// How precise a date is. A vague date is kept vague (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    Day,
    Month,
    Season,
    Year,
    /// No date at all.
    #[serde(other)]
    Unknown,
}

/// When something happened, as precisely as it was said: the first day of
/// the period it names, and how much of that day is meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct When {
    pub on: Option<NaiveDate>,
    pub precision: Precision,
}

impl When {
    pub const UNKNOWN: When = When {
        on: None,
        precision: Precision::Unknown,
    };

    /// Read a date as the owner writes one: `2019-06-14`, `2019-06`,
    /// `summer 2019` (spring, summer, autumn or fall, winter), `2019`, or
    /// nothing. Anything else is refused, never guessed at.
    pub fn parse(text: &str) -> Result<When> {
        let t = text.trim().to_lowercase();
        if t.is_empty() {
            return Ok(When::UNKNOWN);
        }
        let at = |on: NaiveDate, precision| When {
            on: Some(on),
            precision,
        };
        let year = |y: &str| -> Option<i32> {
            (y.len() == 4 && y.bytes().all(|b| b.is_ascii_digit()))
                .then(|| y.parse().ok())
                .flatten()
        };
        if let Some((season, y)) = t.split_once(' ') {
            let month = match season {
                "spring" => 3,
                "summer" => 6,
                "autumn" | "fall" => 9,
                "winter" => 12,
                _ => 0,
            };
            if let (true, Some(y)) = (month > 0, year(y.trim())) {
                if let Some(d) = NaiveDate::from_ymd_opt(y, month, 1) {
                    return Ok(at(d, Precision::Season));
                }
            }
        } else if let Some(y) = year(&t) {
            if let Some(d) = NaiveDate::from_ymd_opt(y, 1, 1) {
                return Ok(at(d, Precision::Year));
            }
        } else if let Ok(d) = NaiveDate::parse_from_str(&t, "%Y-%m-%d") {
            return Ok(at(d, Precision::Day));
        } else if let Ok(d) = NaiveDate::parse_from_str(&format!("{t}-01"), "%Y-%m-%d") {
            return Ok(at(d, Precision::Month));
        }
        bail!(
            "{text:?} is not a date: write 2019-06-14, 2019-06, summer 2019, 2019, \
             or nothing when it is not known"
        )
    }

    /// As precisely as it was said, and no more.
    pub fn render(&self) -> Option<String> {
        let d = self.on?;
        Some(match self.precision {
            Precision::Day => d.to_string(),
            Precision::Month => d.format("%B %Y").to_string(),
            Precision::Season => {
                let season = match d.month() {
                    3..=5 => "spring",
                    6..=8 => "summer",
                    9..=11 => "autumn",
                    _ => "winter",
                };
                format!("{season} {}", d.year())
            }
            Precision::Year => d.year().to_string(),
            Precision::Unknown => return None,
        })
    }
}

/// Someone in her world.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub uid: String,
    pub name: String,
    pub aliases: Vec<String>,
    /// Their role in her life: "older sister", "her advisor".
    pub role: Option<String>,
    pub description: Option<String>,
    /// An image-library character: their portrait (§6.5).
    pub character: Option<String>,
    pub source: Source,
    /// Whether the owner has heard of them (§6.2): until then she
    /// introduces them rather than naming them in passing.
    pub known_to_owner: bool,
}

/// What the owner writes about someone; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PersonEdit {
    pub name: Option<String>,
    pub aliases: Option<Vec<String>>,
    /// `Some(None)` clears it.
    pub role: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub character: Option<Option<String>>,
}

/// One thing that happened on an edge.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeEvent {
    pub uid: String,
    pub person: String,
    /// The other person for a tie between two; `None` for her own tie.
    pub other: Option<String>,
    pub kind: EventKind,
    /// The tie as of this event, when it names one.
    pub tie: Option<Tie>,
    pub label: Option<String>,
    /// How close, when the event places it.
    pub layer: Option<Layer>,
    pub when: When,
    pub context: Option<String>,
    pub source: Source,
    pub recorded_at: String,
}

/// What a new event says; the store adds its id, source and time.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub kind: EventKind,
    pub tie: Option<Tie>,
    pub label: Option<String>,
    pub layer: Option<Layer>,
    pub when: When,
    pub context: Option<String>,
}

impl NewEvent {
    pub fn of(kind: EventKind) -> NewEvent {
        NewEvent {
            kind,
            tie: None,
            label: None,
            layer: None,
            when: When::UNKNOWN,
            context: None,
        }
    }
}

/// Whether an edge stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Paused,
    Ended,
}

/// Warm or tense.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Valence {
    Warm,
    Tense,
}

/// An edge's current state: the fold of its events. A value no event set
/// stays `None`; nothing is defaulted into a closeness or a mood.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub tie: Option<Tie>,
    pub label: Option<String>,
    pub layer: Option<Layer>,
    pub valence: Option<Valence>,
    pub status: Status,
    pub started: Option<When>,
    pub ended: Option<When>,
    pub last: Option<When>,
    /// Events the fold could not read (a kind this binary does not know),
    /// counted rather than guessed at.
    pub unreadable: usize,
    /// Whether any event came from a source that is not clean.
    pub unclean: bool,
}

/// Where an event falls in its edge's history. A dated event falls on its
/// date. An undated `met` or `formed` is still the beginning, so it falls
/// first; any other undated event happened no later than it was recorded,
/// so it falls on the day it was recorded. Ties go by recording time.
fn order(e: &EdgeEvent) -> (NaiveDate, &str) {
    let recorded = || {
        e.recorded_at
            .get(..10)
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .unwrap_or(NaiveDate::MAX)
    };
    let on = match (e.when.on, e.kind) {
        (Some(d), _) => d,
        (None, EventKind::Met | EventKind::Formed) => NaiveDate::MIN,
        (None, _) => recorded(),
    };
    (on, e.recorded_at.as_str())
}

/// Fold one edge's events into its current state, in the order they fell
/// ([`order`]).
///
/// - The tie and label are the latest event's that names them; the layer
///   likewise, and `strengthened` without one moves a ring inward.
/// - `conflict` makes it tense; `strengthened`, `reconciled` and `renewed`
///   make it warm; the rest leave it. A tie's forming says nothing of how
///   it feels (an ex is formed too), so it sets no mood.
/// - `ended` ends it and `paused` pauses it; `met`, `formed`,
///   `strengthened`, `reconciled` and `renewed` make it active again.
/// - It started at its first dated `met` or `formed` (an undated one when
///   none is dated), ended at its latest
///   `ended` while it stays ended, and was last touched at its latest dated
///   event.
pub fn fold(events: &[EdgeEvent]) -> Edge {
    let mut sorted: Vec<&EdgeEvent> = events.iter().collect();
    sorted.sort_by(|a, b| order(a).cmp(&order(b)));
    let mut edge = Edge {
        tie: None,
        label: None,
        layer: None,
        valence: None,
        status: Status::Active,
        started: None,
        ended: None,
        last: None,
        unreadable: 0,
        unclean: false,
    };
    for e in sorted {
        if !e.source.is_clean() {
            edge.unclean = true;
        }
        if e.kind == EventKind::Unknown {
            edge.unreadable += 1;
            continue;
        }
        if let Some(t) = e.tie.filter(|t| *t != Tie::Unknown) {
            edge.tie = Some(t);
        }
        if e.label.is_some() {
            edge.label = e.label.clone();
        }
        match (e.layer, e.kind) {
            (Some(l), _) => edge.layer = Some(l),
            (None, EventKind::Strengthened) => edge.layer = edge.layer.map(Layer::inward),
            _ => {}
        }
        match e.kind {
            EventKind::Conflict => edge.valence = Some(Valence::Tense),
            EventKind::Strengthened | EventKind::Reconciled | EventKind::Renewed => {
                edge.valence = Some(Valence::Warm)
            }
            _ => {}
        }
        match e.kind {
            EventKind::Ended => {
                edge.status = Status::Ended;
                edge.ended = Some(e.when);
            }
            EventKind::Paused => edge.status = Status::Paused,
            EventKind::Met
            | EventKind::Formed
            | EventKind::Strengthened
            | EventKind::Reconciled
            | EventKind::Renewed => {
                edge.status = Status::Active;
                edge.ended = None;
            }
            _ => {}
        }
        // The first start, and a dated one over an undated one: an undated
        // start folds first, and must not hide a date the owner recorded
        // later (review of #801).
        if matches!(e.kind, EventKind::Met | EventKind::Formed)
            && edge.started.is_none_or(|s| s.on.is_none())
        {
            edge.started = Some(e.when);
        }
        if e.when.on.is_some() {
            edge.last = Some(e.when);
        }
    }
    edge
}

/// A few words: trimmed, not empty, capped, no control character.
fn words(what: &str, text: &str, max: usize) -> Result<String> {
    let t = text.trim();
    if t.is_empty() {
        bail!("{what} is empty");
    }
    if t.chars().count() > max {
        bail!("{what} is {} characters; at most {max}", t.chars().count());
    }
    if t.chars().any(char::is_control) {
        bail!("{what} holds a control character");
    }
    Ok(t.to_string())
}

fn optional(what: &str, text: Option<&str>, max: usize) -> Result<Option<String>> {
    match text.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(None),
        Some(t) => words(what, t, max).map(Some),
    }
}

fn aliases_of(list: &[String]) -> Result<Vec<String>> {
    if list.len() > MAX_ALIASES {
        bail!("{} aliases; at most {MAX_ALIASES}", list.len());
    }
    let mut out: Vec<String> = Vec::new();
    for a in list {
        let a = words("an alias", a, MAX_NAME)?;
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&a)) {
            out.push(a);
        }
    }
    Ok(out)
}

fn wire<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("a unit variant serialises as a string"),
    }
}

fn unwire<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).ok()
}

/// Whether this `world.db` has a network yet: one a version-1 binary
/// wrote, read without migrating, has none.
fn has_network(w: &World) -> Result<bool> {
    Ok(w.conn()
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'people'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn person_of(r: &rusqlite::Row) -> rusqlite::Result<Person> {
    let aliases: String = r.get(2)?;
    Ok(Person {
        uid: r.get(0)?,
        name: r.get(1)?,
        // A list this binary cannot read is no aliases, never an error that
        // hides the person.
        aliases: serde_json::from_str(&aliases).unwrap_or_default(),
        role: r.get(3)?,
        description: r.get(4)?,
        character: r.get(5)?,
        source: unwire(&r.get::<_, String>(6)?).unwrap_or(Source::Unknown),
        known_to_owner: r.get(7)?,
    })
}

const PERSON_COLUMNS: &str =
    "uid, name, aliases, role, description, character, source, known_to_owner";

fn event_of(r: &rusqlite::Row) -> rusqlite::Result<EdgeEvent> {
    let layer: Option<u32> = r.get(6)?;
    let on: Option<String> = r.get(7)?;
    let on = on.and_then(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok());
    let raw_precision: String = r.get(8)?;
    let precision = unwire(&raw_precision).unwrap_or(Precision::Unknown);
    let raw_tie: Option<String> = r.get(4)?;
    let kind = unwire(&r.get::<_, String>(3)?).unwrap_or(EventKind::Unknown);
    // A ring, a tie or a date's precision this binary does not know makes
    // the event one it cannot read, counted as such: never "not placed",
    // never a stale tie shown as current, never an event re-dated to the
    // day it was recorded (reviews of #801). `unknown` is how a precision
    // says "no date", so only a word outside the set is unreadable.
    let readable_precision = matches!(
        raw_precision.as_str(),
        "day" | "month" | "season" | "year" | "unknown"
    );
    let unknown_tie = raw_tie
        .as_deref()
        .is_some_and(|t| unwire::<Tie>(t) == Some(Tie::Unknown));
    let kind = match layer {
        Some(n) if Layer::of_size(n).is_none() => EventKind::Unknown,
        _ if unknown_tie || !readable_precision => EventKind::Unknown,
        _ => kind,
    };
    Ok(EdgeEvent {
        uid: r.get(0)?,
        person: r.get(1)?,
        other: r.get(2)?,
        kind,
        tie: raw_tie.map(|t| unwire(&t).unwrap_or(Tie::Unknown)),
        label: r.get(5)?,
        layer: layer.and_then(Layer::of_size),
        // A date without its precision, or a precision without a date, is
        // no date: never a day it was not said to be.
        when: match (on, precision) {
            (Some(d), p) if p != Precision::Unknown => When {
                on: Some(d),
                precision: p,
            },
            _ => When::UNKNOWN,
        },
        context: r.get(9)?,
        source: unwire(&r.get::<_, String>(10)?).unwrap_or(Source::Unknown),
        recorded_at: r.get(11)?,
    })
}

const EVENT_COLUMNS: &str = "uid, person, other, event, tie, label, layer, valid_on, \
                             precision, context, source, recorded_at";

/// Her whole network as it stands: everyone, every event.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Network {
    /// By name, then id.
    pub people: Vec<Person>,
    pub events: Vec<EdgeEvent>,
}

impl Network {
    /// `pdir`'s network, read-only: none yet is an empty network, never an
    /// error and never a file created.
    pub fn read(pdir: &Path) -> Result<Network> {
        let Some(w) = World::existing(pdir)? else {
            return Ok(Network::default());
        };
        if !has_network(&w)? {
            return Ok(Network::default());
        }
        let mut people: Vec<Person> = w
            .conn()
            .prepare(&format!("SELECT {PERSON_COLUMNS} FROM people"))?
            .query_map([], person_of)?
            .collect::<rusqlite::Result<_>>()?;
        people.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.uid.cmp(&b.uid))
        });
        let events = w
            .conn()
            .prepare(&format!(
                "SELECT {EVENT_COLUMNS} FROM edge_events ORDER BY recorded_at, uid"
            ))?
            .query_map([], event_of)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Network { people, events })
    }

    pub fn person(&self, uid: &str) -> Option<&Person> {
        self.people.iter().find(|p| p.uid == uid)
    }

    /// Her own edge to `uid`, folded.
    pub fn her_edge(&self, uid: &str) -> Edge {
        let events: Vec<EdgeEvent> = self
            .events
            .iter()
            .filter(|e| e.person == uid && e.other.is_none())
            .cloned()
            .collect();
        fold(&events)
    }

    /// Every tie between two people, folded, keyed by the ordered pair.
    pub fn ties(&self) -> BTreeMap<(String, String), Edge> {
        let mut by: BTreeMap<(String, String), Vec<EdgeEvent>> = BTreeMap::new();
        for e in &self.events {
            if let Some(o) = &e.other {
                by.entry((e.person.clone(), o.clone()))
                    .or_default()
                    .push(e.clone());
            }
        }
        by.into_iter().map(|(k, v)| (k, fold(&v))).collect()
    }

    /// The one person `who` names: an id or a prefix of one, a name, or an
    /// alias, case aside. Two people of one name is a refusal listing them
    /// by id (§6.4: an alias match is not an identity match).
    pub fn find(&self, who: &str) -> Result<&Person> {
        let w = who.trim();
        if w.len() >= 4 {
            let by_id: Vec<&Person> = self
                .people
                .iter()
                .filter(|p| p.uid.starts_with(&w.to_lowercase()))
                .collect();
            if by_id.len() == 1 {
                return Ok(by_id[0]);
            }
        }
        let named: Vec<&Person> = self
            .people
            .iter()
            .filter(|p| {
                p.name.eq_ignore_ascii_case(w)
                    || p.aliases.iter().any(|a| a.eq_ignore_ascii_case(w))
            })
            .collect();
        match named.as_slice() {
            [one] => Ok(one),
            [] => bail!("no one in her network is named {w:?}"),
            many => bail!(
                "{} people are named {w:?}; name one by id: {}",
                many.len(),
                many.iter()
                    .map(|p| format!(
                        "{} ({})",
                        &p.uid[..8],
                        p.role.as_deref().unwrap_or("no role")
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// An owner-written person: who they are, and her tie to them as its first
/// event (formed, or met when no tie is named).
#[derive(Debug, Clone, PartialEq)]
pub struct NewPerson {
    pub name: String,
    pub aliases: Vec<String>,
    pub role: Option<String>,
    pub description: Option<String>,
    pub character: Option<String>,
    pub tie: Option<Tie>,
    pub label: Option<String>,
    pub layer: Option<Layer>,
    pub since: When,
}

fn checked_event(e: &NewEvent) -> Result<(Option<String>, Option<String>)> {
    if e.kind == EventKind::Unknown {
        bail!("an event needs a kind this binary knows");
    }
    if e.tie == Some(Tie::Unknown) {
        bail!("a tie needs a kind this binary knows");
    }
    Ok((
        optional("a label", e.label.as_deref(), MAX_LABEL)?,
        optional("an event's context", e.context.as_deref(), MAX_CONTEXT)?,
    ))
}

fn insert_event(
    tx: &rusqlite::Transaction,
    person: &str,
    other: Option<&str>,
    e: &NewEvent,
    source: Source,
) -> Result<String> {
    let (label, context) = checked_event(e)?;
    // A tie between two is stored once, under its ordered pair.
    let (person, other) = match other {
        Some(o) if o < person => (o, Some(person)),
        o => (person, o),
    };
    let id = uid();
    tx.execute(
        &format!(
            "INSERT INTO edge_events ({EVENT_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        ),
        params![
            id,
            person,
            other,
            wire(&e.kind),
            e.tie.map(|t| wire(&t)),
            label,
            e.layer.map(Layer::size),
            e.when.on.map(|d| d.to_string()),
            wire(&e.when.precision),
            context,
            wire(&source),
            stamp(),
        ],
    )?;
    Ok(id)
}

fn exists(tx: &rusqlite::Transaction, uid: &str) -> Result<bool> {
    Ok(tx
        .query_row("SELECT 1 FROM people WHERE uid = ?1", [uid], |_| Ok(()))
        .optional()?
        .is_some())
}

/// A new person as the store would write them.
struct Checked {
    name: String,
    aliases: Vec<String>,
    role: Option<String>,
    description: Option<String>,
    character: Option<String>,
    first: NewEvent,
}

fn checked(p: &NewPerson) -> Result<Checked> {
    let first = NewEvent {
        kind: if p.tie.is_some() {
            EventKind::Formed
        } else {
            EventKind::Met
        },
        tie: p.tie,
        label: p.label.clone(),
        layer: p.layer,
        when: p.since,
        context: None,
    };
    let out = Checked {
        name: words("a name", &p.name, MAX_NAME)?,
        aliases: aliases_of(&p.aliases)?,
        role: optional("a role", p.role.as_deref(), MAX_NAME)?,
        description: optional("a description", p.description.as_deref(), MAX_DESCRIPTION)?,
        character: optional("a character", p.character.as_deref(), MAX_NAME)?,
        first,
    };
    checked_event(&out.first)?;
    Ok(out)
}

/// Every check [`add_person`] makes before writing, without writing: so a
/// proposal staged for the owner is one that can be accepted.
pub fn check_new(p: &NewPerson) -> Result<()> {
    checked(p).map(|_| ())
}

/// The owner adds someone to her network, with her tie to them as its
/// first event. Returns their id. `network.md` is rendered again.
pub fn add_person(pdir: &Path, p: &NewPerson) -> Result<String> {
    let Checked {
        name,
        aliases,
        role,
        description,
        character,
        first,
    } = checked(p)?;
    let mut w = World::open(pdir)?;
    let tx = w.conn_mut().transaction()?;
    let count: i64 = tx.query_row("SELECT count(*) FROM people", [], |r| r.get(0))?;
    if count >= MAX_PEOPLE as i64 {
        bail!("her network holds {count} people; at most {MAX_PEOPLE}");
    }
    let id = uid();
    let at = stamp();
    tx.execute(
        "INSERT INTO people (uid, name, aliases, role, description, character, source,
                             known_to_owner, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)",
        params![
            id,
            name,
            serde_json::to_string(&aliases)?,
            role,
            description,
            character,
            wire(&Source::Owner),
            at
        ],
    )?;
    insert_event(&tx, &id, None, &first, Source::Owner)?;
    tx.commit()?;
    render(pdir)?;
    Ok(id)
}

/// The owner changes who someone is. Only the owner writes these fields
/// (§6.2), so an edit replaces them; their edges are untouched.
pub fn edit_person(pdir: &Path, uid: &str, edit: &PersonEdit) -> Result<()> {
    let mut w = World::open(pdir)?;
    let tx = w.conn_mut().transaction()?;
    if !exists(&tx, uid)? {
        bail!("no one in her network has the id {uid}");
    }
    if let Some(n) = &edit.name {
        tx.execute(
            "UPDATE people SET name = ?2 WHERE uid = ?1",
            params![uid, words("a name", n, MAX_NAME)?],
        )?;
    }
    if let Some(a) = &edit.aliases {
        tx.execute(
            "UPDATE people SET aliases = ?2 WHERE uid = ?1",
            params![uid, serde_json::to_string(&aliases_of(a)?)?],
        )?;
    }
    for (column, value, what, max) in [
        ("role", &edit.role, "a role", MAX_NAME),
        (
            "description",
            &edit.description,
            "a description",
            MAX_DESCRIPTION,
        ),
        ("character", &edit.character, "a character", MAX_NAME),
    ] {
        if let Some(v) = value {
            let v = optional(what, v.as_deref(), max)?;
            // `column` is one of three literals above, never input.
            tx.execute(
                &format!("UPDATE people SET {column} = ?2 WHERE uid = ?1"),
                params![uid, v],
            )?;
        }
    }
    tx.execute(
        "UPDATE people SET updated_at = ?2 WHERE uid = ?1",
        params![uid, stamp()],
    )?;
    tx.commit()?;
    render(pdir)
}

/// The owner records something that happened: on her tie to `person`, or
/// on the tie between `person` and `other`. Returns the event's id.
pub fn record(pdir: &Path, person: &str, other: Option<&str>, e: &NewEvent) -> Result<String> {
    if other == Some(person) {
        bail!("a tie is between two different people");
    }
    let mut w = World::open(pdir)?;
    let tx = w.conn_mut().transaction()?;
    for id in std::iter::once(person).chain(other) {
        if !exists(&tx, id)? {
            bail!("no one in her network has the id {id}");
        }
    }
    let id = insert_event(&tx, person, other, e, Source::Owner)?;
    tx.commit()?;
    render(pdir)?;
    Ok(id)
}

/// The owner takes someone out of her network, with every event on every
/// edge they are part of.
pub fn remove_person(pdir: &Path, uid: &str) -> Result<()> {
    let mut w = World::open(pdir)?;
    let tx = w.conn_mut().transaction()?;
    if !exists(&tx, uid)? {
        bail!("no one in her network has the id {uid}");
    }
    tx.execute(
        "DELETE FROM edge_events WHERE person = ?1 OR other = ?1",
        [uid],
    )?;
    tx.execute("DELETE FROM people WHERE uid = ?1", [uid])?;
    tx.commit()?;
    render(pdir)
}

/// The owner takes back an event recorded by mistake. Only an owner event:
/// what a chat contributed goes with `forget`, never one at a time here.
pub fn remove_event(pdir: &Path, event: &str) -> Result<()> {
    let mut w = World::open(pdir)?;
    let tx = w.conn_mut().transaction()?;
    let source: Option<String> = tx
        .query_row(
            "SELECT source FROM edge_events WHERE uid = ?1",
            [event],
            |r| r.get(0),
        )
        .optional()?;
    match source.as_deref().and_then(unwire::<Source>) {
        None if source.is_none() => bail!("no event has the id {event}"),
        Some(Source::Owner) => {}
        _ => bail!("only an event the owner recorded is taken back one at a time"),
    }
    tx.execute("DELETE FROM edge_events WHERE uid = ?1", [event])?;
    tx.commit()?;
    render(pdir)
}

fn tie_words(e: &Edge) -> Option<String> {
    match (&e.label, e.tie) {
        (Some(l), _) => Some(l.clone()),
        (None, Some(t)) => Some(wire(&t)),
        (None, None) => None,
    }
}

/// One person in a line: name, role and tie, the line about them, since
/// when, and how it stands.
fn person_line(p: &Person, e: &Edge) -> String {
    let mut line = format!("**{}**", p.name);
    let mut what: Vec<String> = Vec::new();
    if let Some(r) = &p.role {
        what.push(r.clone());
    }
    if let Some(t) = tie_words(e) {
        if p.role.as_deref() != Some(t.as_str()) {
            what.push(t);
        }
    }
    if !what.is_empty() {
        line.push_str(&format!(" ({})", what.join(", ")));
    }
    if !p.aliases.is_empty() {
        line.push_str(&format!(", also {}", p.aliases.join(", ")));
    }
    if let Some(d) = &p.description {
        line.push_str(&format!(". {d}"));
    }
    let mut notes: Vec<String> = Vec::new();
    if let Some(s) = e.started.and_then(|w| w.render()) {
        notes.push(format!("since {s}"));
    }
    match e.status {
        Status::Ended => notes.push(match e.ended.and_then(|w| w.render()) {
            Some(w) => format!("ended {w}"),
            None => "ended".into(),
        }),
        Status::Paused => notes.push("paused".into()),
        Status::Active => {}
    }
    if let Some(v) = e.valence {
        notes.push(wire(&v));
    }
    // What the fold could not read, or read from a source that is not
    // clean, is said where the owner reads, not only in the JSON.
    if let Some(n) = edge_warning(e) {
        notes.push(n);
    }
    if !notes.is_empty() {
        line.push_str(&format!(" ({})", notes.join(", ")));
    }
    line
}

/// What an edge's fold could not vouch for, in words: events this binary
/// cannot read, and events from a source that is not clean. `None` for an
/// edge folded whole from clean events.
fn edge_warning(e: &Edge) -> Option<String> {
    let mut out = Vec::new();
    if e.unreadable > 0 {
        out.push(format!(
            "{} event{} this version cannot read",
            e.unreadable,
            if e.unreadable == 1 { "" } else { "s" }
        ));
    }
    if e.unclean {
        out.push("some of it from a source that is not clean".to_string());
    }
    (!out.is_empty()).then(|| out.join("; "))
}

impl Network {
    /// Each edge whose fold could not vouch for everything it folded, as a
    /// predicate following the persona's name (`Store::problems`): the owner
    /// reads it beside her other problems rather than in the JSON alone.
    pub fn unreadable_edges(&self) -> Vec<String> {
        let mut out = Vec::new();
        for p in &self.people {
            if let Some(w) = edge_warning(&self.her_edge(&p.uid)) {
                out.push(format!(
                    "has {} in her network, whose tie holds {w}",
                    p.name
                ));
            }
        }
        for ((a, b), e) in self.ties() {
            if let Some(w) = edge_warning(&e) {
                let name = |id: &str| {
                    self.person(id)
                        .map_or("someone", |p| p.name.as_str())
                        .to_string()
                };
                // Alphabetical, as network.md names a tie, so the line reads
                // the same on every install.
                let mut pair = [name(&a), name(&b)];
                pair.sort_by_key(|n| n.to_lowercase());
                out.push(format!(
                    "has a tie between {} and {} in her network that holds {w}",
                    pair[0], pair[1]
                ));
            }
        }
        out
    }
}

/// `network.md`'s text: her people by layer, innermost first, then those
/// not placed, then ended ties, then the ties between people.
pub fn markdown(n: &Network) -> String {
    let mut out = String::from(
        "<!-- Rendered from world.db after every change; edits here are not read back.\n     \
         `mecha persona people` and the People tab change her network. -->\n\n# Her people\n",
    );
    let mut by_layer: BTreeMap<Option<Layer>, Vec<String>> = BTreeMap::new();
    let mut ended: Vec<String> = Vec::new();
    for p in &n.people {
        let e = n.her_edge(&p.uid);
        let line = format!("- {}", person_line(p, &e));
        if e.status == Status::Ended {
            ended.push(line);
        } else {
            by_layer.entry(e.layer).or_default().push(line);
        }
    }
    if n.people.is_empty() {
        out.push_str("\nNo one yet.\n");
    }
    for l in Layer::ALL {
        if let Some(lines) = by_layer.get(&Some(l)) {
            out.push_str(&format!(
                "\n## Closest {}\n\n{}\n",
                l.size(),
                lines.join("\n")
            ));
        }
    }
    if let Some(lines) = by_layer.get(&None) {
        out.push_str(&format!("\n## Not placed\n\n{}\n", lines.join("\n")));
    }
    if !ended.is_empty() {
        out.push_str(&format!(
            "\n## No longer in her life\n\n{}\n",
            ended.join("\n")
        ));
    }
    let mut ties: Vec<String> = n
        .ties()
        .into_iter()
        .filter_map(|((a, b), e)| {
            let (a, b) = (n.person(&a)?, n.person(&b)?);
            // Named in alphabetical order, so the file reads the same on
            // every install and does not churn on a rewrite.
            let (a, b) = if b.name.to_lowercase() < a.name.to_lowercase() {
                (b, a)
            } else {
                (a, b)
            };
            let mut line = format!("- {} and {}", a.name, b.name);
            if let Some(t) = tie_words(&e) {
                line.push_str(&format!(": {t}"));
            }
            match (e.status, e.valence) {
                (Status::Ended, _) => line.push_str(" (ended)"),
                (_, Some(Valence::Tense)) => line.push_str(" (tense)"),
                _ => {}
            }
            Some(line)
        })
        .collect();
    ties.sort();
    if !ties.is_empty() {
        out.push_str(&format!("\n## Between them\n\n{}\n", ties.join("\n")));
    }
    out
}

/// Whether `name` may be someone's portrait: a library character the owner
/// has approved, as `persona.toml`'s own `character` must be. Both doors
/// that write a person (the CLI, the People tab) ask here.
pub fn check_character(lib: &crate::imagelib::Library, name: &str) -> Result<()> {
    match lib.get(crate::imagelib::Kind::Character, name) {
        None => bail!("there is no character `{name}` in the image library"),
        Some(e) if e.status != crate::imagelib::Status::Approved => {
            bail!("`{name}` is a candidate in the image library; approve it first")
        }
        Some(_) => Ok(()),
    }
}

impl Network {
    /// Each person whose portrait link is broken, named: the library
    /// character gone, or no longer approved. Links are by name and
    /// resolved on load, so a link that broke after it was written is
    /// reported, never silently shown as no portrait. A predicate that
    /// follows the persona's name, as `Store::problems`' entries do.
    ///
    /// **The character's name is never quoted.** These lines reach pages a
    /// viewer may see without the library's unlock, and a locked candidate,
    /// or one since removed, would be named there; the owner reads which
    /// character with `mecha persona people show --json` (review of #801).
    pub fn broken_links(&self, lib: &crate::imagelib::Library) -> Vec<String> {
        self.people
            .iter()
            .filter(|p| {
                p.character
                    .as_deref()
                    .is_some_and(|c| check_character(lib, c).is_err())
            })
            .map(|p| {
                format!(
                    "has {} in her network, whose portrait is not an approved image-library \
                     character",
                    p.name
                )
            })
            .collect()
    }
}

/// How much of her system prompt her people may take (PL32): about a
/// tenth of a local chat slot's prompt budget, so a large network never
/// crowds out who she is.
pub const PROMPT_CHARS: usize = 3000;

/// Her people as her own prompt holds them (PL32): in the second person, as
/// `identity.md` is, the closest two layers a line each, everyone else by
/// name and role, ended ties last, then the ties between people. Only
/// people and edges from clean sources: a name or a label a run reading
/// outside content wrote never reaches a system prompt, which every turn of
/// every chat on the version carries. Held to [`PROMPT_CHARS`], dropping
/// from the outside in and saying how many were left out. Empty when there
/// is no one to name.
pub fn prompt_section(n: &Network) -> String {
    let clean: Vec<(&Person, Edge)> = n
        .people
        .iter()
        .filter(|p| p.source.is_clean())
        .map(|p| (p, n.her_edge(&p.uid)))
        .filter(|(_, e)| !e.unclean)
        .collect();
    if clean.is_empty() {
        return String::new();
    }
    let said = |p: &Person, e: &Edge| -> String {
        let mut line = p.name.clone();
        let mut what: Vec<String> = Vec::new();
        if let Some(r) = &p.role {
            what.push(format!("your {r}"));
        }
        if let Some(t) = tie_words(e) {
            if p.role.as_deref() != Some(t.as_str()) {
                what.push(t);
            }
        }
        if !what.is_empty() {
            line.push_str(&format!(" ({})", what.join(", ")));
        }
        line
    };
    let mut close: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    let mut ended: Vec<String> = Vec::new();
    let mut sorted = clean.clone();
    sorted.sort_by_key(|(p, e)| (e.layer.is_none(), e.layer, p.name.to_lowercase()));
    for (p, e) in &sorted {
        if e.status == Status::Ended {
            let when = e
                .ended
                .and_then(|w| w.render())
                .map(|w| format!(", ended {w}"));
            ended.push(format!("{}{}", said(p, e), when.unwrap_or_default()));
            continue;
        }
        if matches!(e.layer, Some(Layer::Five | Layer::Fifteen)) {
            let mut line = format!("- {}", said(p, e));
            if !p.aliases.is_empty() {
                line.push_str(&format!(", also called {}", p.aliases.join(", ")));
            }
            if let Some(d) = &p.description {
                line.push_str(&format!(". {d}"));
            }
            let mut notes: Vec<String> = Vec::new();
            if let Some(s) = e.started.and_then(|w| w.render()) {
                notes.push(format!("since {s}"));
            }
            if e.status == Status::Paused {
                notes.push("you are not in touch just now".into());
            }
            if e.valence == Some(Valence::Tense) {
                notes.push("things are tense between you".into());
            }
            if !notes.is_empty() {
                line.push_str(&format!(" ({})", notes.join("; ")));
            }
            close.push(line);
        } else {
            others.push(said(p, e));
        }
    }
    let names: BTreeMap<&str, &str> = clean
        .iter()
        .map(|(p, _)| (p.uid.as_str(), p.name.as_str()))
        .collect();
    let ties: Vec<String> = n
        .ties()
        .into_iter()
        .filter(|(_, e)| !e.unclean)
        .filter_map(|((a, b), e)| {
            let (a, b) = (names.get(a.as_str())?, names.get(b.as_str())?);
            let mut line = format!("{a} and {b}");
            if let Some(t) = tie_words(&e) {
                line.push_str(&format!(": {t}"));
            }
            match (e.status, e.valence) {
                (Status::Ended, _) => line.push_str(", no longer"),
                (_, Some(Valence::Tense)) => line.push_str(", tense"),
                _ => {}
            }
            Some(line)
        })
        .collect();
    // Laid out innermost first, so a budget drops from the outside in.
    let mut parts: Vec<(&str, Vec<String>, &str)> = Vec::new();
    if !close.is_empty() {
        parts.push(("Closest to you:\n", close, "\n"));
    }
    if !others.is_empty() {
        parts.push(("Others in your life: ", others, ", "));
    }
    if !ended.is_empty() {
        parts.push(("No longer in your life: ", ended, ", "));
    }
    if !ties.is_empty() {
        parts.push(("Between them: ", ties, "; "));
    }
    let total: usize = parts.iter().map(|(_, items, _)| items.len()).sum();
    let mut out = String::new();
    let mut kept = 0;
    'parts: for (head, items, sep) in parts {
        let mut block = String::from(head);
        let mut any = false;
        for item in items {
            let piece = if any { format!("{sep}{item}") } else { item };
            // Room for the block's end and the note of what was left out.
            if out.len() + block.len() + piece.len() + 80 > PROMPT_CHARS {
                if any {
                    out.push_str(&block);
                    out.push_str(".\n");
                }
                break 'parts;
            }
            block.push_str(&piece);
            any = true;
            kept += 1;
        }
        if any {
            out.push_str(&block);
            out.push_str(if sep == "\n" { "\n" } else { ".\n" });
        }
    }
    if kept < total {
        out.push_str(&format!("({} more are not listed here.)\n", total - kept));
    }
    out.trim_end().to_string()
}

/// Write `network.md` from the store as it now stands.
pub fn render(pdir: &Path) -> Result<()> {
    let text = markdown(&Network::read(pdir)?);
    super::write_private(&pdir.join(NETWORK_MD), text.as_bytes())
        .with_context(|| format!("writing {NETWORK_MD}"))
}

impl Person {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.uid,
            "name": self.name,
            "aliases": self.aliases,
            "role": self.role,
            "description": self.description,
            "character": self.character,
            "source": self.source,
            "known_to_owner": self.known_to_owner,
        })
    }
}

impl When {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "on": self.on.map(|d| d.to_string()),
            "precision": self.precision,
            "said": self.render(),
        })
    }
}

impl Edge {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "tie": self.tie,
            "label": self.label,
            "layer": self.layer.map(Layer::size),
            "valence": self.valence,
            "status": self.status,
            "started": self.started.map(|w| w.to_json()),
            "ended": self.ended.map(|w| w.to_json()),
            "last": self.last.map(|w| w.to_json()),
            "unreadable": self.unreadable,
            "unclean": self.unclean,
        })
    }
}

impl EdgeEvent {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.uid,
            "person": self.person,
            "other": self.other,
            "event": self.kind,
            "tie": self.tie,
            "label": self.label,
            "layer": self.layer.map(Layer::size),
            "when": self.when.to_json(),
            "context": self.context,
            "source": self.source,
            "recorded_at": self.recorded_at,
        })
    }
}

impl Network {
    /// Everyone with her edge folded, and every tie between people.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "people": self.people.iter().map(|p| {
                let mut v = p.to_json();
                v["edge"] = self.her_edge(&p.uid).to_json();
                v
            }).collect::<Vec<_>>(),
            "ties": self.ties().into_iter().map(|((a, b), e)| {
                serde_json::json!({ "a": a, "b": b, "edge": e.to_json() })
            }).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::tests_support::*;

    fn pdir() -> std::path::PathBuf {
        let dir = scratch();
        crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
        dir.join("mara")
    }

    fn sister() -> NewPerson {
        NewPerson {
            name: "Ines".into(),
            aliases: vec!["Nessa".into()],
            role: Some("older sister".into()),
            description: Some("Runs a bakery two towns over.".into()),
            character: None,
            tie: Some(Tie::Kin),
            label: Some("sister".into()),
            layer: Some(Layer::Five),
            since: When::UNKNOWN,
        }
    }

    fn at(kind: EventKind, when: &str) -> NewEvent {
        NewEvent {
            when: When::parse(when).unwrap(),
            ..NewEvent::of(kind)
        }
    }

    #[test]
    fn a_date_keeps_the_precision_it_was_given() {
        for (said, precision, rendered) in [
            ("2019-06-14", Precision::Day, Some("2019-06-14")),
            ("2019-06", Precision::Month, Some("June 2019")),
            ("Summer 2019", Precision::Season, Some("summer 2019")),
            ("fall 2019", Precision::Season, Some("autumn 2019")),
            ("winter 2019", Precision::Season, Some("winter 2019")),
            ("2019", Precision::Year, Some("2019")),
            ("", Precision::Unknown, None),
        ] {
            let w = When::parse(said).unwrap();
            assert_eq!(
                (w.precision, w.render().as_deref()),
                (precision, rendered),
                "{said}"
            );
        }
        for bad in [
            "june",
            "19",
            "2019-13",
            "midsummer 2019",
            "2019-06-31",
            "last year",
        ] {
            assert!(When::parse(bad).is_err(), "{bad} was guessed at");
        }
    }

    #[test]
    fn the_fold_is_the_events_in_the_order_they_happened() {
        let ev = |kind, when: &str, recorded: &str| EdgeEvent {
            uid: recorded.into(),
            person: "p".into(),
            other: None,
            kind,
            tie: None,
            label: None,
            layer: None,
            when: When::parse(when).unwrap(),
            context: None,
            source: Source::Owner,
            recorded_at: recorded.into(),
        };
        let mut formed = ev(EventKind::Formed, "2015", "1");
        formed.tie = Some(Tie::Friend);
        formed.layer = Some(Layer::Fifty);
        // Recorded out of order: the fold reads when they happened.
        let events = vec![
            ev(EventKind::Conflict, "2021-03", "2"),
            formed,
            ev(EventKind::Strengthened, "2018", "3"),
        ];
        let e = fold(&events);
        assert_eq!(e.tie, Some(Tie::Friend));
        assert_eq!(
            e.layer,
            Some(Layer::Fifteen),
            "strengthened moves a ring in"
        );
        assert_eq!(e.valence, Some(Valence::Tense), "the conflict came last");
        assert_eq!(e.status, Status::Active);
        assert_eq!(e.started.unwrap().render().as_deref(), Some("2015"));
        assert_eq!(e.last.unwrap().render().as_deref(), Some("March 2021"));

        let mut more = events.clone();
        more.push(ev(EventKind::Ended, "2022", "4"));
        let e = fold(&more);
        assert_eq!(
            (e.status, e.ended.unwrap().render().as_deref()),
            (Status::Ended, Some("2022"))
        );
        more.push(ev(EventKind::Renewed, "2024", "5"));
        let e = fold(&more);
        assert_eq!(
            (e.status, e.ended, e.valence),
            (Status::Active, None, Some(Valence::Warm))
        );

        // An undated start is still the start; an undated later event fell
        // no later than it was recorded, so after a dated one before then.
        let mut start = ev(EventKind::Formed, "", "2026-01-01T00:00:00Z");
        start.tie = Some(Tie::Kin);
        let e = fold(&[
            ev(EventKind::Conflict, "2025", "2026-01-02T00:00:00Z"),
            start.clone(),
        ]);
        assert_eq!(
            e.valence,
            Some(Valence::Tense),
            "the undated start ran after the conflict"
        );
        let e = fold(&[
            start,
            ev(EventKind::Conflict, "2018", "2026-01-02T00:00:00Z"),
            ev(EventKind::Reconciled, "", "2026-01-03T00:00:00Z"),
        ]);
        assert_eq!(
            e.valence,
            Some(Valence::Warm),
            "the undated reconciliation fell when recorded"
        );

        // An undated start does not hide a dated one recorded later.
        let dated = ev(EventKind::Met, "2015", "2026-01-02T00:00:00Z");
        let e = fold(&[ev(EventKind::Formed, "", "2026-01-01T00:00:00Z"), dated]);
        assert_eq!(e.started.unwrap().render().as_deref(), Some("2015"));

        // Nothing set is nothing claimed.
        let bare = fold(&[ev(EventKind::Met, "", "1")]);
        assert_eq!(
            (bare.tie, bare.layer, bare.valence, bare.last),
            (None, None, None, None)
        );
        // An event this binary cannot read is counted, never guessed at,
        // and a source that is not clean marks the edge.
        let mut odd = ev(EventKind::Unknown, "2020", "9");
        odd.source = Source::Unknown;
        let e = fold(&[ev(EventKind::Formed, "2015", "1"), odd]);
        assert_eq!(
            (e.unreadable, e.unclean, e.status),
            (1, true, Status::Active)
        );
    }

    #[test]
    fn the_owner_adds_edits_and_removes_her_people_and_network_md_follows() {
        let pdir = pdir();
        assert_eq!(Network::read(&pdir).unwrap(), Network::default());
        assert!(
            !pdir.join(crate::persona::world::WORLD_DB).exists(),
            "a read created the store"
        );

        let ines = add_person(&pdir, &sister()).unwrap();
        let theo = add_person(
            &pdir,
            &NewPerson {
                name: "Theo".into(),
                aliases: vec![],
                role: Some("lab partner".into()),
                description: None,
                character: None,
                tie: Some(Tie::Colleague),
                label: None,
                layer: Some(Layer::Fifty),
                since: When::parse("autumn 2023").unwrap(),
            },
        )
        .unwrap();
        let n = Network::read(&pdir).unwrap();
        assert_eq!(n.people.len(), 2);
        let p = n.find("nessa").unwrap();
        assert_eq!(
            (p.uid.as_str(), p.source, p.known_to_owner),
            (ines.as_str(), Source::Owner, true)
        );
        let e = n.her_edge(&ines);
        // Forming a tie sets no mood: nothing says how it feels yet.
        assert_eq!(
            (e.tie, e.layer, e.valence),
            (Some(Tie::Kin), Some(Layer::Five), None)
        );

        let md = std::fs::read_to_string(pdir.join(NETWORK_MD)).unwrap();
        assert!(
            md.contains(
                "## Closest 5\n\n- **Ines** (older sister, sister), also Nessa. Runs a bakery"
            ),
            "{md}"
        );
        assert!(
            md.contains("**Theo** (lab partner, colleague) (since autumn 2023)\n"),
            "{md}"
        );

        // A tie between two people is stored once, whichever way it is named.
        record(
            &pdir,
            &theo,
            Some(&ines),
            &NewEvent {
                tie: Some(Tie::Friend),
                ..at(EventKind::Formed, "2024")
            },
        )
        .unwrap();
        record(
            &pdir,
            &ines,
            Some(&theo),
            &at(EventKind::Conflict, "2025-02"),
        )
        .unwrap();
        let n = Network::read(&pdir).unwrap();
        let ties = n.ties();
        assert_eq!(ties.len(), 1);
        assert_eq!(ties.values().next().unwrap().valence, Some(Valence::Tense));
        assert!(std::fs::read_to_string(pdir.join(NETWORK_MD))
            .unwrap()
            .contains("## Between them\n\n- Ines and Theo: friend (tense)"));

        // An edit replaces who someone is; a cleared role is gone.
        edit_person(
            &pdir,
            &theo,
            &PersonEdit {
                role: Some(None),
                name: Some("Theodore".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let n = Network::read(&pdir).unwrap();
        let t = n.find("theodore").unwrap();
        assert_eq!(t.role, None);
        assert!(n.find("theo").is_err());

        // Removing someone takes every edge they are part of.
        remove_person(&pdir, &theo).unwrap();
        let n = Network::read(&pdir).unwrap();
        assert_eq!(n.people.len(), 1);
        assert!(n.ties().is_empty());
        assert!(n
            .events
            .iter()
            .all(|e| e.person == ines && e.other.is_none()));
        assert!(!std::fs::read_to_string(pdir.join(NETWORK_MD))
            .unwrap()
            .contains("Theodore"));
    }

    #[test]
    fn what_the_owner_writes_is_checked_at_the_door() {
        let pdir = pdir();
        let mut p = sister();
        for (tweak, want) in [
            (
                Box::new(|p: &mut NewPerson| p.name = "  ".into()) as Box<dyn Fn(&mut NewPerson)>,
                "a name is empty",
            ),
            (
                Box::new(|p: &mut NewPerson| p.name = "x".repeat(MAX_NAME + 1)),
                "at most",
            ),
            (
                Box::new(|p: &mut NewPerson| p.role = Some("sis\u{7}".into())),
                "control character",
            ),
            (
                Box::new(|p: &mut NewPerson| p.label = Some("x".repeat(MAX_LABEL + 1))),
                "at most",
            ),
            (
                Box::new(|p: &mut NewPerson| p.aliases = vec!["a".into(); MAX_ALIASES + 1]),
                "aliases",
            ),
            (
                Box::new(|p: &mut NewPerson| p.tie = Some(Tie::Unknown)),
                "a tie needs",
            ),
        ] {
            let mut q = p.clone();
            tweak(&mut q);
            let got = format!("{:#}", add_person(&pdir, &q).unwrap_err());
            assert!(got.contains(want), "{got}");
        }
        assert!(Network::read(&pdir).unwrap().people.is_empty());
        // Aliases that differ only in case are one.
        p.aliases = vec!["Nessa".into(), "nessa".into()];
        let id = add_person(&pdir, &p).unwrap();
        assert_eq!(
            Network::read(&pdir).unwrap().person(&id).unwrap().aliases,
            vec!["Nessa"]
        );
        // A tie with oneself, an event on someone not there.
        assert!(record(&pdir, &id, Some(&id), &NewEvent::of(EventKind::Met)).is_err());
        assert!(record(&pdir, "nobody", None, &NewEvent::of(EventKind::Met)).is_err());
        assert!(record(&pdir, &id, None, &NewEvent::of(EventKind::Unknown)).is_err());
    }

    #[test]
    fn two_people_of_one_name_are_named_by_id() {
        let pdir = pdir();
        let a = add_person(&pdir, &sister()).unwrap();
        let mut b = sister();
        b.role = Some("old schoolfriend".into());
        b.aliases = vec![];
        let b = add_person(&pdir, &b).unwrap();
        let n = Network::read(&pdir).unwrap();
        let why = format!("{:#}", n.find("ines").unwrap_err());
        assert!(
            why.contains("2 people") && why.contains(&a[..8]) && why.contains(&b[..8]),
            "{why}"
        );
        assert_eq!(n.find(&b[..8]).unwrap().uid, b);
        assert_eq!(n.find("Nessa").unwrap().uid, a);
    }

    #[test]
    fn only_an_owner_event_is_taken_back_one_at_a_time() {
        let pdir = pdir();
        let id = add_person(&pdir, &sister()).unwrap();
        let ev = record(&pdir, &id, None, &at(EventKind::Conflict, "2025")).unwrap();
        assert_eq!(
            Network::read(&pdir).unwrap().her_edge(&id).valence,
            Some(Valence::Tense)
        );
        remove_event(&pdir, &ev).unwrap();
        assert_eq!(Network::read(&pdir).unwrap().her_edge(&id).valence, None);
        // A row from any other source is forget's, not this door's. There
        // is no chat source yet, so `chat` loads as an unknown one: the
        // fail-closed case.
        let mut w = World::open(&pdir).unwrap();
        w.conn_mut()
            .execute(
                "UPDATE edge_events SET source = 'chat' WHERE person = ?1",
                [&id],
            )
            .unwrap();
        let first = Network::read(&pdir).unwrap().events[0].uid.clone();
        assert!(format!("{:#}", remove_event(&pdir, &first).unwrap_err())
            .contains("only an event the owner"));
        assert!(remove_event(&pdir, "nothing").is_err());
    }

    #[test]
    fn a_world_a_version_one_binary_wrote_has_no_network_yet() {
        let pdir = pdir();
        // A version-1 world.db, as the day sampler wrote it before.
        let conn = rusqlite::Connection::open(pdir.join(crate::persona::world::WORLD_DB)).unwrap();
        conn.execute_batch("CREATE TABLE days (date TEXT); PRAGMA user_version = 1;")
            .unwrap();
        drop(conn);
        assert_eq!(Network::read(&pdir).unwrap(), Network::default());
        // The first write migrates it, and the old table stays.
        add_person(&pdir, &sister()).unwrap();
        assert_eq!(Network::read(&pdir).unwrap().people.len(), 1);
    }

    fn person(name: &str, role: Option<&str>, layer: Option<Layer>) -> NewPerson {
        NewPerson {
            name: name.into(),
            aliases: vec![],
            role: role.map(Into::into),
            description: None,
            character: None,
            tie: Some(Tie::Friend),
            label: None,
            layer,
            since: When::UNKNOWN,
        }
    }

    #[test]
    fn her_prompt_names_the_closest_in_full_and_the_rest_by_name() {
        let pdir = pdir();
        assert_eq!(prompt_section(&Network::read(&pdir).unwrap()), "");
        let ines = add_person(&pdir, &sister()).unwrap();
        let bo = add_person(
            &pdir,
            &person("Bo", Some("climbing partner"), Some(Layer::Fifteen)),
        )
        .unwrap();
        add_person(&pdir, &person("Cal", None, Some(Layer::OneFifty))).unwrap();
        add_person(&pdir, &person("Dee", Some("old flatmate"), None)).unwrap();
        let sam = add_person(&pdir, &person("Sam", None, Some(Layer::Fifty))).unwrap();
        record(&pdir, &sam, None, &at(EventKind::Ended, "2022")).unwrap();
        record(&pdir, &bo, None, &at(EventKind::Conflict, "2025")).unwrap();
        record(
            &pdir,
            &ines,
            Some(&bo),
            &NewEvent {
                tie: Some(Tie::Acquaintance),
                ..at(EventKind::Met, "2023")
            },
        )
        .unwrap();
        let text = prompt_section(&Network::read(&pdir).unwrap());
        assert_eq!(
            text,
            "Closest to you:\n\
             - Ines (your older sister, sister), also called Nessa. Runs a bakery two towns over.\n\
             - Bo (your climbing partner, friend) (things are tense between you)\n\
             Others in your life: Cal (friend), Dee (your old flatmate, friend).\n\
             No longer in your life: Sam (friend), ended 2022.\n\
             Between them: Bo and Ines: acquaintance."
                .replace(
                    "Bo and Ines",
                    if bo < ines {
                        "Bo and Ines"
                    } else {
                        "Ines and Bo"
                    }
                ),
        );
        // Someone from a source that is not clean never reaches it, nor an
        // edge one such event touched.
        let mut w = World::open(&pdir).unwrap();
        w.conn_mut()
            .execute("UPDATE people SET source = 'chat' WHERE uid = ?1", [&sam])
            .unwrap();
        w.conn_mut()
            .execute("UPDATE edge_events SET source = 'chat' WHERE person = ?1 AND other IS NULL AND event = 'conflict'", [&bo])
            .unwrap();
        let text = prompt_section(&Network::read(&pdir).unwrap());
        assert!(!text.contains("Sam") && !text.contains("- Bo"), "{text}");
        assert!(text.contains("Ines"), "{text}");
    }

    #[test]
    fn a_large_network_is_cut_from_the_outside_in_and_says_so() {
        let pdir = pdir();
        add_person(&pdir, &sister()).unwrap();
        for i in 0..120 {
            add_person(
                &pdir,
                &person(
                    &format!("Acquaintance number {i:03}"),
                    Some("someone from the market"),
                    Some(Layer::OneFifty),
                ),
            )
            .unwrap();
        }
        let text = prompt_section(&Network::read(&pdir).unwrap());
        assert!(text.len() <= PROMPT_CHARS, "{}", text.len());
        assert!(
            text.starts_with("Closest to you:\n- Ines"),
            "the closest went first"
        );
        let said = text.lines().last().unwrap();
        assert!(
            said.starts_with('(') && said.ends_with("more are not listed here.)"),
            "{said}"
        );
    }

    #[test]
    fn a_portrait_link_is_checked_in_core_and_a_broken_one_is_named() {
        let dir = scratch();
        let lib_dir = dir.join("imagelib");
        let mut png = Vec::new();
        image::RgbImage::new(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let entry = |name: &str, origin, locked| {
            crate::imagelib::create(
                &lib_dir,
                crate::imagelib::NewEntry {
                    kind: crate::imagelib::Kind::Character,
                    name: name.into(),
                    text: "tall, flour on her sleeves".into(),
                    portrait: Some(png.clone()),
                    source_seed: None,
                    origin,
                    locked,
                },
            )
            .unwrap();
        };
        entry("flourhand", crate::imagelib::Origin::Owner, false);
        // A locked candidate: its name must never reach a page a viewer
        // sees without the unlock.
        entry("veiledone", crate::imagelib::Origin::ModelClean, true);
        let lib = || crate::imagelib::Library::load(&lib_dir).0;
        assert!(check_character(&lib(), "nobody")
            .unwrap_err()
            .to_string()
            .contains("no character"));
        crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
        let pdir = dir.join("mara");
        let mut p = sister();
        p.character = Some("flourhand".into());
        add_person(&pdir, &p).unwrap();
        // Approved (an owner entry is): nothing broken, on the network or
        // the persona.
        check_character(&lib(), "flourhand").unwrap();
        assert!(Network::read(&pdir)
            .unwrap()
            .broken_links(&lib())
            .is_empty());
        // Linked to the locked candidate (written past the CLI's check, as
        // a later un-approval would leave it): broken, and named only by
        // the person.
        let mut theo = sister();
        theo.name = "Theo".into();
        theo.aliases = vec![];
        theo.character = Some("veiledone".into());
        add_person(&pdir, &theo).unwrap();
        let broken = Network::read(&pdir).unwrap().broken_links(&lib());
        assert_eq!(broken.len(), 1, "{broken:?}");
        assert!(
            broken[0].starts_with("has Theo in her network"),
            "{broken:?}"
        );
        // The character goes from the library afterwards: both broken, and
        // no library name in any line the persona's page may show.
        std::fs::remove_dir_all(&lib_dir).unwrap();
        let store = crate::persona::Store::load(&dir);
        let problems = store.problems(store.get("mara").unwrap(), &lib());
        let portraits: Vec<&String> = problems.iter().filter(|p| p.contains("portrait")).collect();
        assert_eq!(portraits.len(), 2, "{problems:?}");
        for line in &problems {
            assert!(
                !line.contains("veiledone") && !line.contains("flourhand"),
                "{line}"
            );
        }
    }

    /// A person written straight into the store, as another binary would.
    fn add_person_conn(c: &rusqlite::Connection, name: &str) -> String {
        let id = uid();
        c.execute(
            "INSERT INTO people (uid, name, aliases, source, known_to_owner, created_at, updated_at)
             VALUES (?1, ?2, '[]', 'owner', 1, '2026-01-01', '2026-01-01')",
            params![id, name],
        )
        .unwrap();
        id
    }

    #[test]
    fn what_a_fold_cannot_read_is_said_where_the_owner_reads() {
        let pdir = pdir();
        let ines = add_person(&pdir, &sister()).unwrap();
        let theo = add_person(
            &pdir,
            &NewPerson {
                name: "Theo".into(),
                aliases: vec![],
                ..sister()
            },
        )
        .unwrap();
        record(&pdir, &ines, None, &at(EventKind::Conflict, "2025")).unwrap();
        record(&pdir, &ines, Some(&theo), &at(EventKind::Met, "2024")).unwrap();
        let n = Network::read(&pdir).unwrap();
        assert!(
            n.unreadable_edges().is_empty(),
            "a whole fold is not a finding"
        );
        // A newer binary wrote an event kind, a ring, and a source this one
        // does not know.
        let mut w = World::open(&pdir).unwrap();
        let c = w.conn_mut();
        c.execute(
            "UPDATE edge_events SET event = 'eloped' WHERE person = ?1 AND event = 'conflict'",
            [&ines],
        )
        .unwrap();
        c.execute("UPDATE edge_events SET layer = 500 WHERE person = ?1 AND event = 'formed' AND other IS NULL", [&theo]).unwrap();
        c.execute(
            "UPDATE edge_events SET source = 'feed' WHERE other IS NOT NULL",
            [],
        )
        .unwrap();
        // A tie and a date precision it does not know either.
        let cal = add_person_conn(c, "Cal");
        let dee = add_person_conn(c, "Dee");
        c.execute(
            "INSERT INTO edge_events (uid, person, other, event, tie, precision, source, recorded_at)
             VALUES ('t1', ?1, NULL, 'formed', 'friend', 'year', 'owner', '2026-01-01T00:00:00Z'),
                    ('t2', ?1, NULL, 'formed', 'spouse', 'unknown', 'owner', '2026-01-02T00:00:00Z'),
                    ('d1', ?2, NULL, 'renewed', NULL, 'year', 'owner', '2026-01-01T00:00:00Z'),
                    ('d2', ?2, NULL, 'ended', NULL, 'decade', 'owner', '2027-01-01T00:00:00Z')",
            [&cal, &dee],
        )
        .unwrap();
        c.execute(
            "UPDATE edge_events SET valid_on = '2024-01-01' WHERE uid IN ('t1', 'd1')",
            [],
        )
        .unwrap();
        c.execute(
            "UPDATE edge_events SET valid_on = '2010-01-01' WHERE uid = 'd2'",
            [],
        )
        .unwrap();
        drop(w);
        render(&pdir).unwrap();
        let n = Network::read(&pdir).unwrap();
        assert_eq!(n.her_edge(&ines).unreadable, 1);
        assert_eq!(
            n.her_edge(&theo).unreadable,
            1,
            "an unknown ring is an unreadable event"
        );
        let md = std::fs::read_to_string(pdir.join(NETWORK_MD)).unwrap();
        assert!(md.contains("1 event this version cannot read"), "{md}");
        // The unknown tie is skipped and counted, never the stale one shown
        // as current-and-whole; the unknown precision is skipped, never
        // re-dated to when it was recorded and folded as ending the tie.
        let cal = n.find("Cal").unwrap().uid.clone();
        let dee = n.find("Dee").unwrap().uid.clone();
        assert_eq!(
            (n.her_edge(&cal).tie, n.her_edge(&cal).unreadable),
            (Some(Tie::Friend), 1)
        );
        assert_eq!(
            (n.her_edge(&dee).status, n.her_edge(&dee).unreadable),
            (Status::Active, 1)
        );
        // An id pasted in capitals is still that id.
        assert_eq!(n.find(&cal[..8].to_uppercase()).unwrap().uid, cal);
        let problems = n.unreadable_edges();
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert!(problems
            .iter()
            .any(|p| p.starts_with("has Ines in her network, whose tie holds 1 event")));
        assert!(problems
            .iter()
            .any(|p| p.contains("tie between") && p.contains("not clean")));
        let store = crate::persona::Store::load(pdir.parent().unwrap());
        let listed = store.problems(store.get("mara").unwrap(), &no_lib());
        assert!(
            listed
                .iter()
                .any(|p| p.contains("this version cannot read")),
            "{listed:?}"
        );
    }

    #[test]
    fn every_closed_set_round_trips_its_wire_name() {
        for t in Tie::ALL {
            assert_eq!(unwire::<Tie>(&wire(&t)), Some(t));
        }
        for k in EventKind::ALL {
            assert_eq!(unwire::<EventKind>(&wire(&k)), Some(k));
        }
        for l in Layer::ALL {
            assert_eq!(Layer::of_size(l.size()), Some(l));
        }
        assert_eq!(unwire::<Tie>("cousin"), Some(Tie::Unknown));
        assert_eq!(unwire::<EventKind>("eloped"), Some(EventKind::Unknown));
    }
}
