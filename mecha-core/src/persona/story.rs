//! The storyteller and the director (`docs/PERSONA-LIFE-DESIGN.md` §4.3), a
//! first build to iterate on: small authored events, storylets, drawn over
//! her sampled days by a director that keeps a rhythm, with no model.
//!
//! **What a storylet is.** An entry in her deck: a title, a size (minor,
//! major, severe), a weight, conditions on her day, and an effect on three
//! short-term levels, energy, stress and mood, each low, normal or high
//! (ruled 2026-11-02). Conditions read what the day already holds: what she
//! is doing, the weekday, the weather where she lives (`persona::weather`),
//! her levels, and whether one of her people with a given tie can be cast.
//!
//! **The director paces them** (§2.4's rhythm):
//! - a floor of minor events, a hassle or an uplift most days
//!   ([`MINOR_CHANCE`], a second one at [`SECOND_MINOR`]);
//! - majors only [`MAJOR_GAP`] days after the last one, at [`MAJOR_CHANCE`]
//!   a day once the gap has passed, forced after [`DROUGHT`] days without
//!   one, and followed by [`QUIET`] quiet days with the minor floor halved;
//! - `severe` events only for a persona whose `[life] intensity` is
//!   `"full"`, under the same pacing as majors;
//! - storylets that cast one of her people weigh [`CAST_WEIGHT`] times.
//!
//! **Her levels drift home.** Each morning a level away from normal steps
//! back with even odds, and the day's events then move it, one step at a
//! time, clamped.
//!
//! **Seeded per persona per day**, like the sampler, so a day's events are
//! the same every time it is asked for. A day up to today is stored when
//! first drawn, and today's is drawn again when what it was drawn from has
//! changed (the deck, the weather, her day); a past day keeps what it was.
//!
//! **Nothing here reaches her prompt.** Events are for the owner to see on
//! the Week tab and in `mecha persona story`. What she is told of them is the
//! teller's step (§4.5), behind a switch of its own (ruled 2026-11-02), so
//! the step 1/2 measures are read on chats this cannot have moved. Only
//! clean people are cast (§4.2), and a cast person is stored by id.
//!
//! **The deck is the owner's.** `story.toml` in her folder, written from
//! the starter for her cell by `mecha persona story init`; until it exists
//! she draws from the starter. The same file holds her `[place]`.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::{Datelike, Days, NaiveDate};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::activity::Activity;
use super::atus::Cell;
use super::network::{Layer, Network, Tie};
use super::weather::{Place, Sky, Weather};

/// Her deck and place, in her folder.
pub const FILE: &str = "story.toml";
/// Bumped when the director's rules change, so today's events redraw.
pub const STORY_VERSION: &str = "story-3";

pub const MINOR_CHANCE: f64 = 0.75;
pub const SECOND_MINOR: f64 = 0.2;
pub const MAJOR_GAP: i64 = 6;
pub const MAJOR_CHANCE: f64 = 0.12;
pub const DROUGHT: i64 = 13;
pub const QUIET: i64 = 2;
pub const CAST_WEIGHT: f64 = 2.0;
/// Days of history a draw reads back: as long as the longest default
/// cooldown, so a storylet resting is seen resting.
pub const LOOKBACK: u64 = 28;
/// The most storylets a deck holds.
pub const MAX_STORYLETS: usize = 200;
/// The longest title.
pub const MAX_TITLE: usize = 80;

/// A short-term level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Low,
    #[default]
    Normal,
    High,
}

impl Level {
    fn step(self, by: i8) -> Level {
        let n = match self {
            Level::Low => -1,
            Level::Normal => 0,
            Level::High => 1,
        } + i32::from(by);
        match n.clamp(-1, 1) {
            -1 => Level::Low,
            0 => Level::Normal,
            _ => Level::High,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Level::Low => "low",
            Level::Normal => "normal",
            Level::High => "high",
        }
    }

    fn from_id(s: &str) -> Option<Level> {
        [Level::Low, Level::Normal, Level::High]
            .into_iter()
            .find(|l| l.id() == s)
    }
}

/// Her three levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Levels {
    pub energy: Level,
    pub stress: Level,
    pub mood: Level,
}

/// A condition on one level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelIs {
    Low,
    Normal,
    High,
    NotLow,
    NotHigh,
}

impl LevelIs {
    fn holds(self, l: Level) -> bool {
        match self {
            LevelIs::Low => l == Level::Low,
            LevelIs::Normal => l == Level::Normal,
            LevelIs::High => l == Level::High,
            LevelIs::NotLow => l != Level::Low,
            LevelIs::NotHigh => l != Level::High,
        }
    }
}

/// How big an event is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    Minor,
    Major,
    /// Holmes–Rahe scale: only under `intensity = "full"`.
    Severe,
}

impl Size {
    pub fn id(self) -> &'static str {
        match self {
            Size::Minor => "minor",
            Size::Major => "major",
            Size::Severe => "severe",
        }
    }

    fn from_id(s: &str) -> Option<Size> {
        [Size::Minor, Size::Major, Size::Severe]
            .into_iter()
            .find(|z| z.id() == s)
    }

    fn weight(self) -> f64 {
        match self {
            Size::Minor => 1.0,
            Size::Major => 3.0,
            Size::Severe => 5.0,
        }
    }

    /// The default rest between two of the same storylet: a minor not
    /// within the week, a major not within the lookback. A severe one is
    /// rested for the whole lookback too, the longest a draw can see.
    pub fn cooldown(self) -> u32 {
        match self {
            Size::Minor => 5,
            Size::Major | Size::Severe => LOOKBACK as u32,
        }
    }
}

/// What an event does to her levels, one step each at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    #[serde(default)]
    pub energy: i8,
    #[serde(default)]
    pub stress: i8,
    #[serde(default)]
    pub mood: i8,
}

/// One storylet, as the deck writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storylet {
    pub id: String,
    /// What happened, for the page; `{who}` names the person cast.
    pub title: String,
    pub size: Size,
    #[serde(default = "one")]
    pub weight: f64,
    /// Any of these activities somewhere in her day.
    #[serde(default)]
    pub doing: Vec<Activity>,
    /// mon..sun, weekdays, weekends.
    #[serde(default)]
    pub days: Vec<String>,
    /// Any of these skies.
    #[serde(default)]
    pub weather: Vec<Sky>,
    /// A high at or above, in °C.
    #[serde(default)]
    pub warmer_than: Option<i16>,
    /// A high at or below, in °C.
    #[serde(default)]
    pub colder_than: Option<i16>,
    #[serde(default)]
    pub energy: Option<LevelIs>,
    #[serde(default)]
    pub stress: Option<LevelIs>,
    #[serde(default)]
    pub mood: Option<LevelIs>,
    /// Casts one of her people with any of these ties.
    #[serde(default)]
    pub with: Vec<Tie>,
    #[serde(default)]
    pub effect: Effect,
    /// Days before it may happen again; by size when left out
    /// ([`Size::cooldown`]).
    #[serde(default)]
    pub cooldown: Option<u32>,
    /// At most one storylet of a group a day, so "slept badly" and "slept
    /// through the night" never share one.
    #[serde(default)]
    pub group: Option<String>,
}

impl Storylet {
    /// Days before it may happen again.
    pub fn rest(&self) -> i64 {
        i64::from(self.cooldown.unwrap_or_else(|| self.size.cooldown()))
    }
}

fn one() -> f64 {
    1.0
}

/// A deck and the place it is drawn at.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Deck {
    pub place: Option<Place>,
    pub storylets: Vec<Storylet>,
}

/// The weekdays a `days` list names, as bits (Monday bit 0); every day when
/// the list is empty.
fn day_bits(days: &[String]) -> Result<u8> {
    if days.is_empty() {
        return Ok(0x7f);
    }
    let mut bits = 0u8;
    for d in days {
        bits |= match d.as_str() {
            "mon" => 1,
            "tue" => 2,
            "wed" => 4,
            "thu" => 8,
            "fri" => 16,
            "sat" => 32,
            "sun" => 64,
            "weekdays" => 0x1f,
            "weekends" => 0x60,
            other => bail!("`{other}` is not a day (mon..sun, weekdays, weekends)"),
        };
    }
    Ok(bits)
}

/// Parse a deck, every storylet checked: refused whole on a key it does not
/// know, a duplicate id, an empty or long title, a weight that is not
/// positive, a day it cannot read, or an effect past one step.
pub fn parse(text: &str) -> Result<Deck> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct File {
        place: Option<Place>,
        #[serde(default)]
        storylet: Vec<Storylet>,
    }
    let f: File = toml::from_str(text).context("story.toml does not load")?;
    if let Some(p) = &f.place {
        p.check()?;
    }
    if f.storylet.len() > MAX_STORYLETS {
        bail!(
            "{} storylets; a deck holds at most {MAX_STORYLETS}",
            f.storylet.len()
        );
    }
    let mut seen = std::collections::HashSet::new();
    for s in &f.storylet {
        let at = || format!("storylet `{}`", s.id);
        if s.id.is_empty()
            || !s
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            bail!(
                "a storylet id is lowercase letters, digits and _: `{}`",
                s.id
            );
        }
        if !seen.insert(s.id.as_str()) {
            bail!("{} appears twice", at());
        }
        let t = s.title.trim();
        if t.is_empty() || t.chars().count() > MAX_TITLE || t.chars().any(char::is_control) {
            bail!(
                "{}: a title is a line of at most {MAX_TITLE} characters",
                at()
            );
        }
        if !(s.weight.is_finite() && s.weight > 0.0) {
            bail!("{}: a weight is above 0", at());
        }
        day_bits(&s.days).with_context(at)?;
        for v in [s.effect.energy, s.effect.stress, s.effect.mood] {
            if !(-1..=1).contains(&v) {
                bail!("{}: an effect moves a level one step, -1 to 1", at());
            }
        }
        if s.title.contains("{who}") && s.with.is_empty() {
            bail!("{}: `{{who}}` needs a `with` to cast someone", at());
        }
        if s.with.contains(&Tie::Unknown) {
            bail!("{}: a tie this binary does not know", at());
        }
    }
    Ok(Deck {
        place: f.place,
        storylets: f.storylet,
    })
}

const SHARED: &str = include_str!("story/shared.toml");
const STUDENT: &str = include_str!("story/student.toml");
const WORKER: &str = include_str!("story/worker.toml");
const PARENT: &str = include_str!("story/parent.toml");
const RETIRED: &str = include_str!("story/retired.toml");

/// The starter deck for `cell`: the shared everyday deck and her cell's.
pub fn starter(cell: Cell) -> String {
    let own = match cell {
        Cell::Student => STUDENT,
        Cell::FullTime | Cell::PartTime => WORKER,
        Cell::FullTimeParent => PARENT,
        Cell::AtHome => "",
        Cell::Retired => RETIRED,
    };
    format!("{HEAD}\n{SHARED}\n{own}")
}

const HEAD: &str = "\
# Her storylets: small events the director draws over her days. Yours to
# edit; nothing here reaches her chats yet. Conditions are all optional and
# all must hold: doing (activities), days, weather (clear, cloudy, fog,
# drizzle, rain, snow, storm), warmer_than / colder_than (°C), energy /
# stress / mood (low, normal, high, not_low, not_high), with (ties: kin,
# romantic, ex, friend, colleague, mentor, neighbour, acquaintance, rival).
# effect moves energy / stress / mood one step (-1 or 1). size is minor,
# major or severe (severe only with [life] intensity = \"full\"). cooldown
# is the days before it may happen again (5 for a minor, 28 otherwise), and
# at most one storylet of a group happens a day.
#
# Where she lives, for her weather (fetched by `mecha persona story weather`):
# [place]
# name = \"Hanover, NH\"
# lat = 43.70
# lon = -72.29
";

/// Write her deck from `cell`'s starter, owner-only. Refused when she has
/// one: the deck is the owner's, never written over.
pub fn init(pdir: &Path, cell: Cell) -> Result<std::path::PathBuf> {
    let path = pdir.join(FILE);
    if path.exists() {
        bail!("she already has {FILE}; edit it");
    }
    super::write_private(&path, starter(cell).as_bytes())?;
    Ok(path)
}

/// Her deck: `story.toml` when she has one, else her cell's starter.
pub fn deck(pdir: &Path, cell: Cell) -> Result<(Deck, String)> {
    let path = pdir.join(FILE);
    let text = if path.exists() {
        std::fs::read_to_string(&path).with_context(|| format!("reading {FILE}"))?
    } else {
        starter(cell)
    };
    Ok((parse(&text)?, text))
}

/// One of her people the director may cast.
#[derive(Debug, Clone, PartialEq)]
pub struct Castable {
    pub uid: String,
    pub tie: Tie,
    pub layer: Option<Layer>,
}

/// Her clean, active people with a tie, as the director may cast them.
pub fn castable(n: &Network) -> Vec<Castable> {
    n.people
        .iter()
        .filter(|p| p.source.is_clean())
        .filter_map(|p| {
            let e = n.her_edge(&p.uid);
            (e.status == super::network::Status::Active && !e.unclean).then_some(())?;
            Some(Castable {
                uid: p.uid.clone(),
                tie: e.tie?,
                layer: e.layer,
            })
        })
        .collect()
}

fn closeness(layer: Option<Layer>) -> f64 {
    match layer {
        Some(Layer::Five) => 1.0,
        Some(Layer::Fifteen) => 0.6,
        Some(Layer::Fifty) => 0.3,
        Some(Layer::OneFifty) => 0.1,
        None => 0.0,
    }
}

/// One drawn event.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub storylet: String,
    pub title: String,
    pub size: Size,
    /// Who was cast, by id.
    pub people: Vec<String>,
    pub effect: Effect,
    /// Importance to her (§9): size × (1 + closeness of who was there) ×
    /// (1 + how far it moved her, in thirds).
    pub importance: f64,
}

/// What a day's draw reads.
#[derive(Debug, Clone)]
pub struct Today<'a> {
    pub persona: &'a str,
    pub date: NaiveDate,
    pub doing: Vec<Activity>,
    pub weather: Option<Weather>,
    pub levels: Levels,
    pub people: &'a [Castable],
    /// Days since her last major (or severe) event; `None` when none in the
    /// lookback, which reads as a drought.
    pub since_major: Option<i64>,
    /// Storylets drawn in the lookback, with how many days ago.
    pub recent: Vec<(String, i64)>,
    pub severe_open: bool,
}

/// splitmix64, seeded per persona per day.
struct Draw(u64);

impl Draw {
    fn new(persona: &str, date: NaiveDate) -> Draw {
        let d = Sha256::digest(format!("mecha-story\0{persona}\0{date}").as_bytes());
        Draw(u64::from_le_bytes(d[..8].try_into().expect("8 bytes")))
    }

    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    /// An index drawn by weight, or `None` when every weight is 0.
    fn pick(&mut self, weights: &[f64]) -> Option<usize> {
        let total: f64 = weights.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut x = self.next() * total;
        for (i, w) in weights.iter().enumerate() {
            if x < *w {
                return Some(i);
            }
            x -= w;
        }
        weights.iter().rposition(|w| *w > 0.0)
    }
}

/// The morning's levels: each away from normal steps back with even odds.
pub fn drift(levels: Levels, draw: &mut impl FnMut() -> f64) -> Levels {
    let home = |l: Level, x: f64| match l {
        Level::Normal => l,
        _ if x < 0.5 => Level::Normal,
        _ => l,
    };
    Levels {
        energy: home(levels.energy, draw()),
        stress: home(levels.stress, draw()),
        mood: home(levels.mood, draw()),
    }
}

fn apply(levels: Levels, e: Effect) -> Levels {
    Levels {
        energy: levels.energy.step(e.energy),
        stress: levels.stress.step(e.stress),
        mood: levels.mood.step(e.mood),
    }
}

/// Whether `s` may fire in `cx` with `levels`, and whom it would cast from.
fn eligible<'a>(s: &Storylet, cx: &Today<'a>, levels: Levels) -> Option<Vec<&'a Castable>> {
    let weekday = cx.date.weekday().num_days_from_monday();
    if day_bits(&s.days).ok()? & (1 << weekday) == 0 {
        return None;
    }
    if !s.doing.is_empty() && !s.doing.iter().any(|a| cx.doing.contains(a)) {
        return None;
    }
    // Unknown weather satisfies no weather condition.
    if !s.weather.is_empty() || s.warmer_than.is_some() || s.colder_than.is_some() {
        let w = cx.weather?;
        if !s.weather.is_empty() && !s.weather.contains(&w.sky) {
            return None;
        }
        if s.warmer_than.is_some_and(|t| w.high_c < t)
            || s.colder_than.is_some_and(|t| w.high_c > t)
        {
            return None;
        }
    }
    for (cond, l) in [
        (s.energy, levels.energy),
        (s.stress, levels.stress),
        (s.mood, levels.mood),
    ] {
        if cond.is_some_and(|c| !c.holds(l)) {
            return None;
        }
    }
    if s.size == Size::Severe && !cx.severe_open {
        return None;
    }
    if s.with.is_empty() {
        return Some(Vec::new());
    }
    let cast: Vec<&Castable> = cx
        .people
        .iter()
        .filter(|p| s.with.contains(&p.tie))
        .collect();
    (!cast.is_empty()).then_some(cast)
}

/// The day's events and her evening levels: the director's draw over `deck`
/// in `cx`. Pure: the same inputs, the same day.
pub fn draw_day(deck: &Deck, cx: &Today) -> (Vec<Event>, Levels, Levels) {
    let mut rng = Draw::new(cx.persona, cx.date);
    let morning = drift(cx.levels, &mut || rng.next());
    let mut levels = morning;
    let mut events = Vec::new();
    let mut used: Vec<&str> = Vec::new();
    let quiet = cx.since_major.is_some_and(|d| d <= QUIET);
    // A major first, under the director's pacing.
    let gap_passed = cx.since_major.is_none_or(|d| d >= MAJOR_GAP);
    let forced = cx.since_major.is_none_or(|d| d >= DROUGHT);
    let major_roll = rng.next();
    if gap_passed && (forced || major_roll < MAJOR_CHANCE) {
        if let Some(e) = pick(deck, cx, levels, &mut rng, &used, |z| z != Size::Minor) {
            levels = apply(levels, e.effect);
            used.push(deck_id(deck, &e));
            events.push(e);
        }
    }
    // The minor floor, halved in a quiet spell.
    let scale = if quiet { 0.5 } else { 1.0 };
    let first = rng.next();
    let second = rng.next();
    for (roll, chance) in [(first, MINOR_CHANCE), (second, MINOR_CHANCE * SECOND_MINOR)] {
        if roll >= chance * scale {
            break;
        }
        if let Some(e) = pick(deck, cx, levels, &mut rng, &used, |z| z == Size::Minor) {
            levels = apply(levels, e.effect);
            used.push(deck_id(deck, &e));
            events.push(e);
        }
    }
    (events, morning, levels)
}

fn deck_id<'d>(deck: &'d Deck, e: &Event) -> &'d str {
    deck.storylets
        .iter()
        .find(|s| s.id == e.storylet)
        .map_or("", |s| s.id.as_str())
}

fn pick(
    deck: &Deck,
    cx: &Today,
    levels: Levels,
    rng: &mut Draw,
    used: &[&str],
    size_ok: impl Fn(Size) -> bool,
) -> Option<Event> {
    let taken: Vec<&str> = deck
        .storylets
        .iter()
        .filter(|s| used.contains(&s.id.as_str()))
        .filter_map(|s| s.group.as_deref())
        .collect();
    let options: Vec<(&Storylet, Vec<&Castable>)> = deck
        .storylets
        .iter()
        .filter(|s| size_ok(s.size) && !used.contains(&s.id.as_str()))
        .filter(|s| s.group.as_deref().is_none_or(|g| !taken.contains(&g)))
        // Resting: drawn more recently than its cooldown.
        .filter(|s| {
            !cx.recent
                .iter()
                .any(|(id, ago)| *id == s.id && *ago < s.rest())
        })
        .filter_map(|s| eligible(s, cx, levels).map(|c| (s, c)))
        .collect();
    let weights: Vec<f64> = options
        .iter()
        .map(|(s, c)| s.weight * if c.is_empty() { 1.0 } else { CAST_WEIGHT })
        .collect();
    let (s, cast) = &options[rng.pick(&weights)?];
    // Who, weighted by closeness, with a floor so an unplaced tie is drawn.
    let who = if cast.is_empty() {
        None
    } else {
        let w: Vec<f64> = cast.iter().map(|c| 0.1 + closeness(c.layer)).collect();
        rng.pick(&w).map(|i| cast[i])
    };
    let moved = f64::from(
        s.effect.energy.unsigned_abs()
            + s.effect.stress.unsigned_abs()
            + s.effect.mood.unsigned_abs(),
    );
    Some(Event {
        storylet: s.id.clone(),
        title: s.title.trim().to_string(),
        size: s.size,
        people: who.map(|c| vec![c.uid.clone()]).unwrap_or_default(),
        effect: s.effect,
        importance: s.size.weight()
            * (1.0 + who.map_or(0.0, |c| closeness(c.layer)))
            * (1.0 + moved / 3.0),
    })
}

/// A day of her story, as the page and the CLI show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryDay {
    pub date: NaiveDate,
    pub weather: Option<WeatherOut>,
    pub morning: Levels,
    pub evening: Levels,
    pub events: Vec<Event>,
    /// Stored as drawn (past or today), or drawn and not kept (ahead).
    pub kept: bool,
}

/// The weather as served: closed fields only.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct WeatherOut {
    pub sky: Sky,
    pub high_c: i16,
    pub low_c: i16,
}

impl From<Weather> for WeatherOut {
    fn from(w: Weather) -> Self {
        WeatherOut {
            sky: w.sky,
            high_c: w.high_c,
            low_c: w.low_c,
        }
    }
}

/// What a day's events were drawn from, as one digest.
fn inputs(
    deck_text: &str,
    weather: Option<Weather>,
    day_digest: &str,
    people: &[Castable],
    severe_open: bool,
) -> String {
    let mut h = Sha256::new();
    h.update(STORY_VERSION.as_bytes());
    h.update([0]);
    // `[life] intensity`: flipping it redraws today, so a severe event
    // never outlives the switch that allowed it (review of #814).
    h.update(if severe_open { b"severe" } else { b"capped" });
    h.update([0]);
    h.update(deck_text.as_bytes());
    h.update([0]);
    h.update(format!("{weather:?}").as_bytes());
    h.update([0]);
    h.update(day_digest.as_bytes());
    h.update([0]);
    for p in people {
        h.update(format!("{}:{:?}:{:?};", p.uid, p.tie, p.layer).as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Her story from `from` to `to`, inclusive: each day drawn on the evening
/// of the one before, stored up to `today`. Refused by name when `[life]
/// world` is off or her days cannot be drawn.
pub fn story(
    store_dir: &Path,
    name: &str,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<StoryDay>> {
    let store = super::Store::load(store_dir);
    let Some(p) = store.get(name) else {
        bail!("no persona named `{name}`");
    };
    if !p.settings.life.world {
        bail!("[life] world is off for `{name}`; her days are not drawn");
    }
    let pdir = store_dir.join(&p.name);
    let routine = super::routine::load(&pdir)?;
    let Some(cell) = routine.as_ref().and_then(|r| r.cell) else {
        bail!("`{name}` has no routine naming her kind of week, so no day is drawn");
    };
    let (deck, deck_text) = deck(&pdir, cell)?;
    let people = if p.settings.life.people {
        castable(&Network::read(&pdir)?)
    } else {
        Vec::new()
    };
    let severe_open = p.settings.life.intensity == super::Intensity::Full;
    let mut world = super::world::World::open(&pdir)?;
    // Start a lookback before `from`, so the first day shown has a
    // morning and a history.
    let start = from - Days::new(LOOKBACK);
    let mut levels = Levels::default();
    let mut last_major: Option<NaiveDate> = None;
    let mut drawn: Vec<(String, NaiveDate)> = Vec::new();
    let mut out = Vec::new();
    let mut d = start;
    while d <= to {
        let weather = world.weather(d)?;
        let stored = world.story_day(d)?;
        // A stored past day is what it was, and needs nothing drawn: only
        // today and the days not yet kept read her day, so a week view
        // loads her store a handful of times, not once per lookback day
        // (review of #814).
        let past = stored
            .as_ref()
            .filter(|_| d < today)
            .map(|(sd, _)| sd.clone());
        let sd = if let Some(sd) = past {
            sd
        } else {
            let day = super::world::day(store_dir, &p.name, d, today)?;
            let doing: Vec<Activity> = day.stretches.iter().map(|s| s.activity).collect();
            let digest = inputs(
                &deck_text,
                weather,
                &format!("{:?}", day.stretches),
                &people,
                severe_open,
            );
            if let Some((sd, _)) = stored.filter(|(_, was)| *was == digest) {
                sd
            } else {
                let cx = Today {
                    persona: &p.name,
                    date: d,
                    doing,
                    weather,
                    levels,
                    people: &people,
                    since_major: last_major.map(|m| (d - m).num_days()),
                    recent: drawn
                        .iter()
                        .map(|(id, on)| (id.clone(), (d - *on).num_days()))
                        .collect(),
                    severe_open,
                };
                let (events, morning, evening) = draw_day(&deck, &cx);
                let sd = StoryDay {
                    date: d,
                    weather: weather.map(WeatherOut::from),
                    morning,
                    evening,
                    events,
                    kept: d <= today,
                };
                if d <= today {
                    world.set_story_day(&sd, &digest)?;
                }
                sd
            }
        };
        // Weather is a fact, not a draw: shown as it stands now, so a day
        // drawn before its weather arrived still shows it.
        let mut sd = sd;
        sd.weather = weather.map(WeatherOut::from);
        levels = sd.evening;
        drawn.extend(sd.events.iter().map(|e| (e.storylet.clone(), d)));
        if sd.events.iter().any(|e| e.size != Size::Minor) {
            last_major = Some(d);
        }
        if d >= from {
            out.push(sd);
        }
        d = d + Days::new(1);
    }
    Ok(out)
}

/// The names of her people by id, for showing a `{who}`.
pub fn names(pdir: &Path) -> Result<BTreeMap<String, String>> {
    Ok(Network::read(pdir)?
        .people
        .into_iter()
        // Only a clean person's name is shown, as only a clean person is
        // cast: a row that stopped being clean reads "someone" (review of
        // #814).
        .filter(|p| p.source.is_clean())
        .map(|p| (p.uid, p.name))
        .collect())
}

/// An event's title with `{who}` filled from `names`, or "someone" for an id
/// no longer in her network.
pub fn title(e: &Event, names: &BTreeMap<String, String>) -> String {
    let who = e
        .people
        .first()
        .map(|u| names.get(u).map_or("someone", String::as_str))
        .unwrap_or("someone");
    e.title.replace("{who}", who)
}

/// Days as the CLI's `--json` and the Week tab read them: each event with
/// its title as shown, `{who}` filled from `names`.
pub fn to_json(days: &[StoryDay], names: &BTreeMap<String, String>) -> serde_json::Value {
    serde_json::Value::Array(
        days.iter()
            .map(|d| {
                let mut j = serde_json::to_value(d).expect("closed fields");
                if let Some(evs) = j["events"].as_array_mut() {
                    for (e, ev) in d.events.iter().zip(evs) {
                        ev["shown"] = serde_json::json!(title(e, names));
                    }
                }
                j
            })
            .collect(),
    )
}

pub(super) fn level_of(s: &str) -> Option<Level> {
    Level::from_id(s)
}

pub(super) fn size_of(s: &str) -> Option<Size> {
    Size::from_id(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn every_starter_deck_parses() {
        for cell in Cell::ALL {
            let deck = parse(&starter(cell)).unwrap_or_else(|e| panic!("{cell:?}: {e:#}"));
            assert!(deck.storylets.len() >= 10, "{cell:?}");
            assert!(
                deck.storylets.iter().any(|s| s.size == Size::Major),
                "{cell:?}"
            );
            assert!(deck.place.is_none());
        }
    }

    #[test]
    fn a_deck_is_refused_whole_on_what_it_cannot_read() {
        let ok = "[[storylet]]\nid = \"a\"\ntitle = \"t\"\nsize = \"minor\"\n";
        parse(ok).unwrap();
        for (bad, want) in [
            ("[[storylet]]\nid = \"a\"\ntitle = \"t\"\nsize = \"huge\"\n", "does not load"),
            (&format!("{ok}{ok}"), "twice"),
            ("[[storylet]]\nid = \"A\"\ntitle = \"t\"\nsize = \"minor\"\n", "lowercase"),
            ("[[storylet]]\nid = \"a\"\ntitle = \"t\"\nsize = \"minor\"\ndays = [\"funday\"]\n", "not a day"),
            ("[[storylet]]\nid = \"a\"\ntitle = \"t\"\nsize = \"minor\"\neffect = { mood = 2 }\n", "one step"),
            ("[[storylet]]\nid = \"a\"\ntitle = \"met {who}\"\nsize = \"minor\"\n", "needs a `with`"),
            ("[[storylet]]\nid = \"a\"\ntitle = \"t\"\nsize = \"minor\"\nsparkle = true\n", "does not load"),
            ("[place]\nname = \"x\"\nlat = 99.0\nlon = 0.0\n", "lat is"),
        ] {
            let got = format!("{:#}", parse(bad).unwrap_err());
            assert!(got.contains(want), "{bad}: {got}");
        }
    }

    fn deck(text: &str) -> Deck {
        parse(text).unwrap()
    }

    fn cx<'a>(date: &str, people: &'a [Castable]) -> Today<'a> {
        Today {
            persona: "mara",
            date: d(date),
            doing: vec![Activity::Class, Activity::Commute],
            weather: Some(Weather {
                date: d(date),
                sky: Sky::Rain,
                high_c: 8,
                low_c: 3,
            }),
            levels: Levels::default(),
            people,
            since_major: Some(1),
            recent: Vec::new(),
            severe_open: false,
        }
    }

    #[test]
    fn conditions_all_hold_or_the_storylet_is_out() {
        let deck = deck(
            r#"
[[storylet]]
id = "soaked"
title = "soaked on the way"
size = "minor"
doing = ["commute"]
weather = ["rain"]
[[storylet]]
id = "sunny"
title = "sun"
size = "minor"
weather = ["clear"]
[[storylet]]
id = "gym"
title = "gym"
size = "minor"
doing = ["exercise"]
[[storylet]]
id = "sat"
title = "sat"
size = "minor"
days = ["sat"]
[[storylet]]
id = "friend"
title = "coffee with {who}"
size = "minor"
with = ["friend"]
"#,
        );
        let none: Vec<Castable> = vec![];
        let c = cx("2026-11-02", &none); // a Monday
        let fires: Vec<&str> = deck
            .storylets
            .iter()
            .filter(|s| eligible(s, &c, c.levels).is_some())
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(fires, vec!["soaked"]);
        // Unknown weather satisfies no weather condition.
        let mut blind = cx("2026-11-02", &none);
        blind.weather = None;
        assert!(eligible(&deck.storylets[0], &blind, blind.levels).is_none());
        // With a friend in her network, the cast storylet opens.
        let friends = vec![Castable {
            uid: "f".repeat(32),
            tie: Tie::Friend,
            layer: Some(Layer::Five),
        }];
        let c = cx("2026-11-02", &friends);
        assert!(eligible(&deck.storylets[4], &c, c.levels).is_some());
    }

    #[test]
    fn a_day_is_the_same_every_time_and_moves_her_levels() {
        let deck = deck(&starter(Cell::Student));
        let none: Vec<Castable> = vec![];
        let c = cx("2026-11-02", &none);
        let a = draw_day(&deck, &c);
        let b = draw_day(&deck, &c);
        assert_eq!(a, b);
        let (events, morning, evening) = a;
        let moved = events.iter().fold(morning, |l, e| apply(l, e.effect));
        assert_eq!(moved, evening);
    }

    #[test]
    fn the_director_keeps_a_rhythm() {
        // Run a deck through a season and read the pacing back.
        let deck = deck(&starter(Cell::FullTime));
        let none: Vec<Castable> = vec![];
        let mut levels = Levels::default();
        let mut last_major: Option<NaiveDate> = None;
        let mut majors = Vec::new();
        let mut minor_days = 0;
        let mut recent: Vec<(String, NaiveDate)> = Vec::new();
        let start = d("2026-01-01");
        let n = 365;
        for i in 0..n {
            let date = start + Days::new(i);
            let c = Today {
                persona: "mara",
                date,
                doing: vec![
                    Activity::Work,
                    Activity::Commute,
                    Activity::Eating,
                    Activity::Exercise,
                ],
                weather: None,
                levels,
                people: &none,
                since_major: last_major.map(|m| (date - m).num_days()),
                recent: recent
                    .iter()
                    .map(|(id, on): &(String, NaiveDate)| (id.clone(), (date - *on).num_days()))
                    .collect(),
                severe_open: false,
            };
            let (events, _, evening) = draw_day(&deck, &c);
            levels = evening;
            recent.extend(events.iter().map(|e| (e.storylet.clone(), date)));
            if events.iter().any(|e| e.size == Size::Minor) {
                minor_days += 1;
            }
            if events.iter().any(|e| e.size != Size::Minor) {
                if let Some(m) = last_major {
                    assert!(
                        (date - m).num_days() >= MAJOR_GAP,
                        "a major {date} too soon after {m}"
                    );
                }
                last_major = Some(date);
                majors.push(date);
            }
            assert!(
                events.iter().all(|e| e.size != Size::Severe),
                "capped: no severe"
            );
        }
        // A floor most days, and majors every week or two.
        assert!(
            minor_days > n * 55 / 100 && minor_days < n * 85 / 100,
            "{minor_days}"
        );
        assert!(majors.len() >= 20 && majors.len() <= 52, "{}", majors.len());
        let mut last: std::collections::HashMap<&str, NaiveDate> = Default::default();
        for (id, on) in &recent {
            if let Some(prev) = last.insert(id.as_str(), *on) {
                let rest = deck.storylets.iter().find(|s| s.id == *id).unwrap().rest();
                assert!(
                    (*on - prev).num_days() >= rest,
                    "{id} again after {} days",
                    (*on - prev).num_days()
                );
            }
        }
        for w in majors.windows(2) {
            assert!(
                (w[1] - w[0]).num_days() <= DROUGHT,
                "a drought past {DROUGHT}: {:?}",
                w
            );
        }
    }

    #[test]
    fn a_group_happens_at_most_once_a_day() {
        let deck = deck(
            "[[storylet]]\nid = \"bad\"\ntitle = \"slept badly\"\nsize = \"minor\"\ngroup = \"sleep\"\ncooldown = 0\n\
             [[storylet]]\nid = \"good\"\ntitle = \"slept well\"\nsize = \"minor\"\ngroup = \"sleep\"\ncooldown = 0\n\
             [[storylet]]\nid = \"keys\"\ntitle = \"lost keys\"\nsize = \"minor\"\ncooldown = 0\n",
        );
        let none: Vec<Castable> = vec![];
        let mut two = 0;
        for i in 0..400 {
            let mut c = cx("2026-01-01", &none);
            c.date = d("2026-01-01") + Days::new(i);
            let (events, _, _) = draw_day(&deck, &c);
            let sleep = events.iter().filter(|e| e.storylet != "keys").count();
            assert!(sleep <= 1, "{:?}", events);
            two += usize::from(events.len() == 2);
        }
        assert!(
            two > 0,
            "a second minor does happen, from outside the group"
        );
    }

    #[test]
    fn severe_needs_full_intensity() {
        let deck = deck("[[storylet]]\nid = \"s\"\ntitle = \"t\"\nsize = \"severe\"\n");
        let none: Vec<Castable> = vec![];
        let mut c = cx("2026-11-02", &none);
        assert!(eligible(&deck.storylets[0], &c, c.levels).is_none());
        c.severe_open = true;
        assert!(eligible(&deck.storylets[0], &c, c.levels).is_some());
    }

    /// A store with one persona, `mara`, her world on and a student's week.
    fn store() -> std::path::PathBuf {
        use crate::persona::tests_support::*;
        let dir = scratch();
        crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        let toml = dir.join("mara/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        std::fs::write(&toml, text.replace("world     = false", "world     = true")).unwrap();
        std::fs::write(dir.join("mara/routine.toml"), "cell = \"student\"\n").unwrap();
        dir
    }

    #[test]
    fn a_story_is_stored_as_drawn_and_today_redraws_when_its_inputs_change() {
        use super::super::world::World;
        let dir = store();
        let today = d("2026-11-02");
        let from = today - Days::new(6);
        let week = story(&dir, "mara", from, today, today).unwrap();
        assert_eq!(week.len(), 7);
        assert!(week.iter().all(|s| s.kept && s.weather.is_none()));
        assert_eq!(story(&dir, "mara", from, today, today).unwrap(), week);
        // Weather arrives: today redraws with it, the past keeps what it was.
        let mut w = super::super::world::World::open(&dir.join("mara")).unwrap();
        w.set_weather(&[
            Weather {
                date: today,
                sky: Sky::Snow,
                high_c: -2,
                low_c: -9,
            },
            Weather {
                date: from,
                sky: Sky::Rain,
                high_c: 9,
                low_c: 4,
            },
        ])
        .unwrap();
        drop(w);
        let again = story(&dir, "mara", from, today, today).unwrap();
        for (a, w) in again[..6].iter().zip(&week[..6]) {
            assert_eq!(
                (&a.events, a.morning, a.evening),
                (&w.events, w.morning, w.evening),
                "the past is what it was"
            );
        }
        assert_eq!(
            again[0].weather,
            Some(WeatherOut {
                sky: Sky::Rain,
                high_c: 9,
                low_c: 4
            }),
            "weather is a fact, shown on a day drawn before it arrived"
        );
        assert_eq!(
            again[6].weather,
            Some(WeatherOut {
                sky: Sky::Snow,
                high_c: -2,
                low_c: -9
            })
        );
        // Her own deck: today draws only from it.
        std::fs::write(
            dir.join("mara").join(FILE),
            "[[storylet]]\nid = \"only\"\ntitle = \"the only thing\"\nsize = \"minor\"\n",
        )
        .unwrap();
        let mine = story(&dir, "mara", from, today, today).unwrap();
        assert_eq!(mine[1..6], again[1..6]);
        assert!(
            mine[6].events.iter().all(|e| e.storylet == "only"),
            "{:?}",
            mine[6].events
        );
        // Intensity is an input: flipping it redraws today, never the past.
        std::fs::write(
            dir.join("mara").join(FILE),
            "[[storylet]]\nid = \"only\"\ntitle = \"the only thing\"\nsize = \"minor\"\n\
             [[storylet]]\nid = \"grave\"\ntitle = \"a grave thing\"\nsize = \"severe\"\n",
        )
        .unwrap();
        let capped = story(&dir, "mara", today, today, today).unwrap();
        let toml = dir.join("mara/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        let full = if text.contains("intensity") {
            text.replace("\"capped\"", "\"full\"")
        } else {
            text.replace("[life]\n", "[life]\nintensity = \"full\"\n")
        };
        std::fs::write(&toml, &full).unwrap();
        let opened = World::open(&dir.join("mara"))
            .unwrap()
            .story_day(today)
            .unwrap()
            .unwrap()
            .1;
        let after = story(&dir, "mara", today, today, today).unwrap();
        let redrawn = World::open(&dir.join("mara"))
            .unwrap()
            .story_day(today)
            .unwrap()
            .unwrap()
            .1;
        assert_ne!(
            opened, redrawn,
            "intensity changes what today was drawn from"
        );
        assert!(capped[0].events.iter().all(|e| e.size != Size::Severe));
        let _ = after;
        std::fs::write(&toml, &text).unwrap();
        // A day ahead is drawn, never stored.
        let ahead = story(
            &dir,
            "mara",
            today + Days::new(1),
            today + Days::new(1),
            today,
        )
        .unwrap();
        assert!(!ahead[0].kept);
        let w = super::super::world::World::open(&dir.join("mara")).unwrap();
        assert!(w.story_day(today + Days::new(1)).unwrap().is_none());
        assert_eq!(w.story_day(today).unwrap().unwrap().0, mine[6]);
    }

    #[test]
    fn a_story_needs_her_world_and_a_kind_of_week() {
        let dir = store();
        std::fs::write(dir.join("mara/routine.toml"), "").unwrap();
        let today = d("2026-11-02");
        let why = format!(
            "{:#}",
            story(&dir, "mara", today, today, today).unwrap_err()
        );
        assert!(why.contains("kind of week"), "{why}");
    }

    #[test]
    fn a_title_names_who_was_cast() {
        let e = Event {
            storylet: "x".into(),
            title: "coffee with {who}".into(),
            size: Size::Minor,
            people: vec!["u1".into()],
            effect: Effect::default(),
            importance: 1.0,
        };
        let mut names = BTreeMap::new();
        assert_eq!(title(&e, &names), "coffee with someone");
        names.insert("u1".to_string(), "Dan".to_string());
        assert_eq!(title(&e, &names), "coffee with Dan");
    }
}
