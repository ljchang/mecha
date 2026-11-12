//! The today-block (`docs/PERSONA-LIFE-DESIGN.md` §4.5): what she is doing
//! now, from her sampled day (`persona::world`), as a run note of its own.
//!
//! It arms taint at the fold by its stem, as the recall block does: two
//! stems, because notes arm at every run start and one stem arming both would
//! make every chat with a day untrusted. Both arm `private`, since the block
//! will carry memory of conversations with the owner (stated plans, people
//! she mentioned). The untrusted stem is chosen the moment any folded row's
//! source is not clean. Its own note, never joined to another: a stem matches
//! only at the start of a note, so a second fold's stem inside one block would
//! never arm on a resume. Registered in `Taint::arm_for_content`,
//! `message::is_recorded_note` and `agent::is_harness_voice`.

use super::activity::Activity;
use super::atus::Company;
use super::sampler;
use super::story::{self, Level, StoryDay};
use super::world::Now;
use anyhow::{Context, Result};
use chrono::{NaiveDateTime, Timelike};
use std::collections::BTreeMap;

/// The block's opening when every row in it is clean.
pub const TODAY_STEM: &str = "(What you are doing right now, from the harness";
/// The opening when any row in it came from outside.
pub const UNTRUSTED_TODAY_STEM: &str =
    "(What you are doing right now, some of it first read from outside, from the harness";
/// The teller's event line, and the marker the initiative measure reads.
pub const EVENTS_HEADING: &str = "What happened";
/// The shared instruction that closes either shape of the today-block.
const NOTE_TAIL: &str = "Let it colour what you say when it fits; it is never news to announce, and where it disagrees with who you are, who you are wins.)";

/// `(private, untrusted)` for a today-block, or `None` when `text` is not
/// one.
pub fn stem_of(text: &str) -> Option<(bool, bool)> {
    let t = text.trim_start();
    // The untrusted stem first: it is the longer, more specific one.
    if t.starts_with(UNTRUSTED_TODAY_STEM) {
        Some((true, true))
    } else if t.starts_with(TODAY_STEM) {
        Some((true, false))
    } else {
        None
    }
}

/// How the note says an activity, after "You are".
fn doing(a: Activity) -> &'static str {
    match a {
        Activity::Sleep => "asleep",
        Activity::PersonalCare => "getting ready",
        Activity::Eating => "eating",
        Activity::Cooking => "cooking",
        Activity::Housework => "doing housework",
        Activity::Shopping => "shopping",
        Activity::Errands => "running errands",
        Activity::Caring => "looking after someone",
        Activity::Work => "working",
        Activity::Class => "in class",
        Activity::Studying => "studying",
        Activity::Commute => "commuting",
        Activity::Travel => "travelling",
        Activity::Socializing => "socializing",
        Activity::Tv => "watching TV",
        Activity::Music => "listening to music",
        Activity::Games => "playing games",
        Activity::Computer => "using a computer",
        Activity::Hobbies => "on a hobby",
        Activity::Reading => "reading",
        Activity::Relaxing => "relaxing",
        Activity::Outing => "out",
        Activity::Exercise => "exercising",
        Activity::SportsEvent => "at a sports event",
        Activity::Religion => "at a religious service",
        Activity::Volunteering => "volunteering",
    }
}

/// How the note names a non-household kind of company. Household kinds are
/// said only through the names `world::now` resolved from the owner's life
/// profile (`persona::life`), never as a relation: the kinds come from the
/// diary a stretch was drawn from, a stranger's, and a note naming the
/// diary's child made a persona written with none invent one (2026-11-03).
fn who(c: Company) -> Option<&'static str> {
    match c {
        Company::Alone => Some("alone"),
        Company::Partner
        | Company::Child
        | Company::Parent
        | Company::Family
        | Company::Housemate => None,
        Company::Friend => Some("friends"),
        Company::Acquaintance => Some("people you know"),
        Company::Colleague => Some("colleagues"),
        Company::Public => Some("people you don't know"),
    }
}

/// Whether a turn tells yesterday's events and today's morning levels.
pub fn before_noon(at: NaiveDateTime) -> bool {
    // A day runs 4am to 4am (`sampler::DAY_START`): before 4am is the
    // evening of the day before, not its morning (review of #836).
    (4..12).contains(&at.hour())
}

/// The day `at` falls in, which runs 4am to 4am: before 4am, the calendar
/// day before, as `world::now` reads it.
pub fn diary_date(at: NaiveDateTime) -> chrono::NaiveDate {
    if at.hour() < 4 {
        at.date().pred_opt().unwrap_or(at.date())
    } else {
        at.date()
    }
}

fn fahrenheit(c: i16) -> i16 {
    (f64::from(c) * 9.0 / 5.0 + 32.0).round() as i16
}

/// The teller lines for a turn at `at`. Before noon, `yesterday` supplies
/// the events while `today` still supplies the morning levels and today's
/// weather; after noon, today's events and evening levels are used.
///
/// This fold is clean by construction: event titles are from the owner's
/// deck, names are the clean People-tab names [`story::names`] admits, and
/// weather is only a closed [`super::weather::Sky`] plus bounded integers
/// (PERSONA-LIFE-DESIGN.md §8.2, shelf A by capacity).
pub fn teller_lines(
    at: NaiveDateTime,
    today: &StoryDay,
    yesterday: Option<&StoryDay>,
    names: &BTreeMap<String, String>,
) -> Result<String> {
    Ok(teller(at, today, yesterday, names)?.lines())
}

/// The teller's lines, each on its own so a chat can stop repeating one it
/// has already used ([`Ledger`]). [`teller_lines`] is these joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Teller {
    /// "What happened today: …", always present.
    pub events: String,
    /// The words an owner's message or her reply shares with the events
    /// when it is about them: the titles' longer words.
    pub event_words: Vec<String>,
    /// "You feel …", when a level is away from normal.
    pub feeling: Option<String>,
    /// "Today's weather is …", when the day has weather.
    pub weather: Option<String>,
    /// The closed sky the weather line names.
    pub sky: Option<super::weather::Sky>,
}

impl Teller {
    /// Every line, in the order the note has always carried them.
    pub fn lines(&self) -> String {
        self.only(&Item::ALL)
    }

    /// The lines for `items`, in the note's order; empty when none is held.
    pub fn only(&self, items: &[Item]) -> String {
        Item::ALL
            .iter()
            .filter(|i| items.contains(i))
            .filter_map(|i| self.line(*i))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn line(&self, item: Item) -> Option<&str> {
        match item {
            Item::Events => Some(self.events.as_str()),
            Item::Feeling => self.feeling.as_deref(),
            Item::Weather => self.weather.as_deref(),
        }
    }
}

/// [`teller_lines`], kept apart by line.
pub fn teller(
    at: NaiveDateTime,
    today: &StoryDay,
    yesterday: Option<&StoryDay>,
    names: &BTreeMap<String, String>,
) -> Result<Teller> {
    let morning = before_noon(at);
    let events = if morning {
        yesterday.context("yesterday's story is missing")?
    } else {
        today
    };
    let happened = if events.events.is_empty() {
        "nothing notable".to_string()
    } else {
        events
            .events
            .iter()
            .map(|event| story::title(event, names))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let events_line = format!(
        "{EVENTS_HEADING} {}: {happened}.",
        if morning { "yesterday" } else { "today" }
    );
    let event_words = if events.events.is_empty() {
        Vec::new()
    } else {
        content_words(&happened)
    };

    let levels = if morning {
        today.morning
    } else {
        today.evening
    };
    let mut feeling = Vec::new();
    let mut feeling_line = None;
    match levels.energy {
        Level::Low => feeling.push("low on energy"),
        Level::High => feeling.push("full of energy"),
        Level::Normal => {}
    }
    match levels.stress {
        Level::Low => feeling.push("relaxed"),
        Level::High => feeling.push("stressed"),
        Level::Normal => {}
    }
    match levels.mood {
        Level::Low => feeling.push("in a low mood"),
        Level::High => feeling.push("in a good mood"),
        Level::Normal => {}
    }
    if !feeling.is_empty() {
        let said = match feeling.split_last() {
            Some((last, [])) => last.to_string(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => unreachable!("checked non-empty"),
        };
        feeling_line = Some(format!("You feel {said}."));
    }
    let weather = today.weather.map(|w| {
        format!(
            "Today's weather is {}, {}–{} °C ({}–{} °F).",
            w.sky.id(),
            w.low_c,
            w.high_c,
            fahrenheit(w.low_c),
            fahrenheit(w.high_c)
        )
    });
    Ok(Teller {
        events: events_line,
        event_words,
        feeling: feeling_line,
        weather,
        sky: today.weather.map(|w| w.sky),
    })
}

/// One of the teller's lines, as the [`Ledger`] keeps them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Item {
    Events,
    Feeling,
    Weather,
}

impl Item {
    /// In the note's order.
    pub const ALL: [Item; 3] = [Item::Events, Item::Feeling, Item::Weather];
}

/// How many turns a line rides unsaid before it is dropped: the turn it is
/// new, and one more for a first turn that was about something else.
pub const OFFERS: u8 = 2;

/// Words in an owner's message that ask about the weather, beside the sky's
/// own ([`sky_words`]). Never the everyday ones ("a hot take", "cold
/// feet", "outside the box"): each false match re-offers the line, which is
/// the error this ledger exists to stop (review of #867).
const WEATHER_ASKED: &[&str] = &[
    "weather",
    "forecast",
    "temperature",
    "degrees",
    "umbrella",
    "chilly",
    "freezing",
    "humid",
    "windy",
];
/// The weather words that count as her having said it: the unambiguous
/// ones, so "a hot coffee" or "step outside" is not the weather told.
const WEATHER_SAID: &[&str] = &["weather", "forecast", "degrees", "umbrella"];
/// An owner's message that asks after her day, which is what the events
/// line answers.
const DAY_ASKED: &[&[&str]] = &[
    &["your", "day"],
    &["your", "morning"],
    &["your", "afternoon"],
    &["your", "evening"],
    &["your", "night"],
    &["your", "weekend"],
    &["what", "happened"],
    &["what's", "new"],
    &["anything", "new"],
    &["up", "to"],
    &["been", "doing"],
    &["did", "you", "do"],
    &["how", "was"],
    &["how", "did"],
];
/// An owner's message that asks how she is, which is what the feeling line
/// answers.
const FEELING_ASKED: &[&[&str]] = &[
    &["how", "are", "you"],
    &["how're", "you"],
    &["how", "you", "doing"],
    &["how", "do", "you", "feel"],
    &["you", "feeling"],
    &["you", "feel"],
    &["you", "ok"],
    &["you", "okay"],
    &["you", "alright"],
    &["how", "are", "things"],
];
/// What she says when she tells a level, by the words the line used.
const FEELING_SAID: &[(&str, &[&str])] = &[
    (
        "low on energy",
        &[
            "tired",
            "exhausted",
            "drained",
            "wiped",
            "sleepy",
            "worn",
            "energy",
            "beat",
            "knackered",
        ],
    ),
    (
        "full of energy",
        &[
            "energy",
            "energetic",
            "energised",
            "energized",
            "buzzing",
            "wired",
        ],
    ),
    (
        "relaxed",
        &["relaxed", "relaxing", "calm", "chill", "mellow"],
    ),
    (
        "stressed",
        &[
            "stressed",
            "stress",
            "stressful",
            "tense",
            "frazzled",
            "overwhelmed",
            "anxious",
        ],
    ),
    (
        "in a low mood",
        &["mood", "down", "low", "blue", "flat", "sad"],
    ),
    (
        "in a good mood",
        &["mood", "happy", "cheerful", "great", "upbeat", "glad"],
    ),
];

/// How a sky is said in a message, by the stems a word may open with.
fn sky_words(sky: super::weather::Sky) -> &'static [&'static str] {
    use super::weather::Sky;
    match sky {
        Sky::Clear => &["sunn", "sunsh", "sunli"],
        Sky::Cloudy => &["cloud", "overcast"],
        Sky::Fog => &["fog", "mist", "haz"],
        Sky::Drizzle => &["drizzl", "rain"],
        Sky::Rain => &["rain", "downpour"],
        Sky::Snow => &["snow", "flurr", "blizzard"],
        Sky::Storm => &["storm", "thunder", "lightning"],
    }
}

fn lowered(text: &str) -> Vec<String> {
    // A phone or a Mac types the curly apostrophe, which is the plain one.
    text.replace('\u{2019}', "'")
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The longer words of event titles: what a message shares with them when
/// it is about them. Five letters and up, which leaves out the titles'
/// connecting words without a stop list.
fn content_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = lowered(text)
        .into_iter()
        .filter(|w| w.chars().count() >= 5)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn has_phrase(words: &[String], phrases: &[&[&str]]) -> bool {
    phrases.iter().any(|p| {
        words
            .windows(p.len())
            .any(|w| w.iter().zip(p.iter()).all(|(a, b)| a == b))
    })
}

fn has_any(words: &[String], set: &[&str]) -> bool {
    words.iter().any(|w| set.contains(&w.as_str()))
}

fn has_stem(words: &[String], stems: &[&str]) -> bool {
    words.iter().any(|w| stems.iter().any(|s| w.starts_with(s)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// The line as first offered. For the weather it is also the line
    /// every later offer of the day repeats, so a forecast refreshed
    /// mid-chat never contradicts what she already said.
    text: String,
    event_words: Vec<String>,
    sky: Option<super::weather::Sky>,
    offered: u8,
    said: bool,
    /// Rode in the last note sent, so her next reply is read for it.
    last: bool,
}

/// What one chat has already been told of her day, so a line the
/// conversation did not pick up is not offered turn after turn (owner,
/// 2026-11-12: rain in 24 of 31 replies of one chat, 2 of them asked).
///
/// A line rides until her reply has said it, or it has been offered
/// [`OFFERS`] times, and again on any turn the owner's message is about it.
/// What she said stays in the history, which is where a follow-up comes
/// from. A new diary day starts over; within a day a changed events or
/// feeling line (the noon turn) is a new line, and the weather is the day's
/// first.
///
/// Held per chat by the process serving it and never stored: a restart
/// offers each line once more. This is the chat's half of
/// PERSONA-LIFE-DESIGN.md §4.5's told-ledger; the nightly `told` flag on
/// the world's rows is not written here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    day: Option<chrono::NaiveDate>,
    entries: BTreeMap<Item, Entry>,
}

impl Ledger {
    /// The teller lines this turn's note carries, or `None` for none.
    ///
    /// `day` is the diary day ([`diary_date`]); `owner` is the message being
    /// answered; `last_reply` is what she said since the last note, which is
    /// read for the lines that note carried.
    pub fn offer(
        &mut self,
        day: chrono::NaiveDate,
        teller: &Teller,
        owner: &str,
        last_reply: &str,
    ) -> Option<String> {
        if self.day != Some(day) {
            self.day = Some(day);
            self.entries.clear();
        }
        let reply = lowered(last_reply);
        for (item, e) in &mut self.entries {
            if std::mem::take(&mut e.last) && mentions(*item, e, &reply, false) {
                e.said = true;
            }
        }
        let asked = lowered(owner);
        let mut riding = Vec::new();
        for item in Item::ALL {
            let Some(line) = teller.line(item) else {
                continue;
            };
            let fresh = || Entry {
                text: line.to_string(),
                event_words: teller.event_words.clone(),
                sky: teller.sky,
                offered: 0,
                said: false,
                last: false,
            };
            let e = self.entries.entry(item).or_insert_with(fresh);
            // The weather is the day's first reading; the others are new
            // lines when they change.
            if item != Item::Weather && e.text != line {
                *e = fresh();
            }
            let due = !e.said && e.offered < OFFERS;
            if due || mentions(item, e, &asked, true) {
                e.offered = e.offered.saturating_add(1);
                e.last = true;
                riding.push((item, e.text.clone()));
            }
        }
        if riding.is_empty() {
            return None;
        }
        Some(
            riding
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

/// Whether `words` are about `item`: an owner's message asking (`asking`),
/// or her reply having said it.
fn mentions(item: Item, e: &Entry, words: &[String], asking: bool) -> bool {
    match item {
        Item::Events => {
            words.iter().any(|w| e.event_words.contains(w))
                || (asking && has_phrase(words, DAY_ASKED))
        }
        Item::Feeling => {
            if asking && has_phrase(words, FEELING_ASKED) {
                return true;
            }
            FEELING_SAID
                .iter()
                .filter(|(said, _)| e.text.contains(said))
                .any(|(_, set)| has_any(words, set))
        }
        Item::Weather => {
            e.sky.is_some_and(|s| has_stem(words, sky_words(s)))
                || has_any(words, if asking { WEATHER_ASKED } else { WEATHER_SAID })
        }
    }
}

/// The today-block for `now`. The times are the owner's clock, which is the
/// one her day runs on.
pub fn note(now: &Now) -> String {
    note_with_events(now, None)
}

/// [`note`] with teller lines after the doing-now sentence. `None` is the
/// events-off and story-error path and is byte-for-byte the old note.
pub fn note_with_events(now: &Now, events: Option<&str>) -> String {
    let s = &now.stretch;
    let clock = |m: u32| format!("{:02}:{:02}", (m / 60) % 24, m % 60);
    let mut said = format!("You are {}", doing(s.activity));
    // The routine's own name for it, when it gave one (the owner's words in
    // an approved file, held to the parser's limits on the way back out of
    // the store).
    if let Some(label) = s.anchor.as_deref().filter(|l| *l != s.activity.id()) {
        said.push_str(&format!(" ({label})"));
    }
    // A stretch that opens the day began before 4am, on the day before, and
    // one that closes it runs past the next 4am: neither edge is a time.
    if s.start != sampler::DAY_START {
        said.push_str(&format!(", since {}", clock(s.start)));
    }
    if s.end != sampler::DAY_END {
        said.push_str(&format!(", until about {}", clock(s.end)));
    }
    match s.company.as_deref() {
        None | Some([]) => {}
        Some([Company::Alone]) => said.push_str(", alone"),
        Some(kinds) => {
            let mut names = now.with_names.clone();
            names.extend(
                kinds
                    .iter()
                    .filter(|k| **k != Company::Alone)
                    .filter_map(|k| who(*k))
                    .map(str::to_string),
            );
            // With only household kinds, nothing is said of company at all:
            // "alone" would be as wrong as "with your child".
            if let Some(listed) = match names.split_last() {
                Some((last, [])) => Some(last.to_string()),
                Some((last, rest)) => Some(format!("{} and {last}", rest.join(", "))),
                None => None,
            } {
                said.push_str(&format!(", with {listed}"));
            }
        }
    }
    let stem = if now.source.is_clean() {
        TODAY_STEM
    } else {
        UNTRUSTED_TODAY_STEM
    };
    match events.filter(|e| !e.trim().is_empty()) {
        None => format!("{stem} — your own day, drawn from your routine. {said}. {NOTE_TAIL}"),
        Some(events) => {
            format!(
                "{stem} — your own day, drawn from your routine. {said}.\n{events}\n{NOTE_TAIL}"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::sampler::Stretch;
    use crate::persona::world::Source;
    use chrono::NaiveDate;

    fn now(stretch: Stretch, source: Source) -> Now {
        Now {
            date: NaiveDate::from_ymd_opt(2026, 3, 3).unwrap(),
            minute: stretch.start,
            stretch,
            source,
            with_names: Vec::new(),
        }
    }

    fn stretch(activity: Activity, start: u32, end: u32) -> Stretch {
        Stretch {
            activity,
            start,
            end,
            company: None,
            anchor: None,
        }
    }

    fn story_day(
        date: &str,
        title: &str,
        morning: story::Levels,
        evening: story::Levels,
    ) -> StoryDay {
        StoryDay {
            date: date.parse().unwrap(),
            weather: Some(story::WeatherOut {
                sky: crate::persona::weather::Sky::Rain,
                high_c: 9,
                low_c: 4,
            }),
            morning,
            evening,
            events: vec![story::Event {
                storylet: "small_win".into(),
                title: title.into(),
                size: story::Size::Minor,
                people: vec!["theo".into()],
                effect: story::Effect::default(),
                importance: 1.0,
            }],
            kept: true,
        }
    }

    #[test]
    fn a_clean_day_opens_with_the_clean_stem_and_reads_as_one_sentence() {
        let mut s = stretch(Activity::Exercise, 7 * 60, 8 * 60 + 30);
        s.anchor = Some("swim at the lido".into());
        s.company = Some(vec![Company::Friend, Company::Colleague]);
        let text = note(&now(s, Source::Sampler));
        assert_eq!(stem_of(&text), Some((true, false)));
        assert!(
            text.contains(
                "You are exercising (swim at the lido), since 07:00, until about 08:30, \
                 with friends and colleagues."
            ),
            "{text}"
        );
        assert!(
            text.contains("where it disagrees with who you are, who you are wins"),
            "{text}"
        );
    }

    #[test]
    fn a_row_from_an_unknown_source_takes_the_untrusted_stem() {
        let text = note(&now(stretch(Activity::Work, 540, 1020), Source::Unknown));
        assert!(text.starts_with(UNTRUSTED_TODAY_STEM));
        assert_eq!(stem_of(&text), Some((true, true)));
        // The owner's own rows are clean.
        let owner = note(&now(stretch(Activity::Work, 540, 1020), Source::Owner));
        assert_eq!(stem_of(&owner), Some((true, false)));
    }

    #[test]
    fn the_day_edges_are_not_times_and_an_unnamed_anchor_says_nothing_twice() {
        // Asleep since before 4am, until after the next one.
        let mut s = stretch(Activity::Sleep, sampler::DAY_START, sampler::DAY_END);
        s.anchor = Some("sleep".into());
        let text = note(&now(s, Source::Sampler));
        assert!(text.contains("You are asleep. Let it"), "{text}");
        // Three kinds of company read as a list.
        let mut s = stretch(Activity::Eating, 19 * 60, 20 * 60);
        s.company = Some(vec![
            Company::Friend,
            Company::Acquaintance,
            Company::Colleague,
        ]);
        let text = note(&now(s, Source::Sampler));
        assert!(
            text.contains("with friends, people you know and colleagues."),
            "{text}"
        );
        // Past midnight, the clock wraps.
        let mut late = stretch(Activity::Socializing, 22 * 60, 25 * 60 + 15);
        late.company = Some(vec![Company::Alone]);
        let text = note(&now(late, Source::Sampler));
        assert!(
            text.contains("since 22:00, until about 01:15, alone."),
            "{text}"
        );
    }

    #[test]
    fn a_household_relation_from_the_diary_is_never_said() {
        // The diary's child, family and strangers: only the strangers.
        let mut s = stretch(Activity::Eating, 22 * 60 + 51, 23 * 60 + 21);
        s.company = Some(vec![Company::Child, Company::Family, Company::Public]);
        let text = note(&now(s, Source::Sampler));
        assert!(
            text.contains("until about 23:21, with people you don't know."),
            "{text}"
        );
        // Only household kinds: nothing said of company, not "alone".
        for kinds in [
            vec![Company::Partner],
            vec![Company::Child, Company::Parent],
            vec![Company::Family, Company::Housemate],
        ] {
            let mut s = stretch(Activity::Cooking, 18 * 60, 19 * 60);
            s.company = Some(kinds.clone());
            let text = note(&now(s, Source::Sampler));
            assert!(
                text.contains("until about 19:00. Let it"),
                "{kinds:?}: {text}"
            );
            for word in ["partner", "child", "parent", "family", "housemate", "alone"] {
                assert!(!text.contains(word), "{kinds:?} said {word}: {text}");
            }
        }
    }

    #[test]
    fn a_household_kind_kept_by_the_profile_names_the_person() {
        let mut s = stretch(Activity::Eating, 19 * 60, 20 * 60);
        s.company = Some(vec![Company::Housemate]);
        let mut at = now(s, Source::Sampler);
        at.with_names = vec!["Theo".into()];
        let text = note(&at);
        assert!(text.contains("with Theo."), "{text}");
        assert!(!text.contains("your housemate"), "{text}");
    }

    #[test]
    fn teller_lines_are_present_only_when_supplied() {
        let at = now(
            stretch(Activity::Reading, 10 * 60 + 7, 11 * 60 + 13),
            Source::Sampler,
        );
        let without = note(&at);
        assert!(!without.contains(EVENTS_HEADING));
        assert_eq!(
            without,
            // Built from the note's own pieces, as `note` builds it.
            format!(
                "{TODAY_STEM} — your own day, drawn from your routine. {}. {NOTE_TAIL}",
                "You are reading, since 10:07, until about 11:13"
            ),
            "the events-off note stays byte-identical"
        );
        let text = note_with_events(&at, Some("What happened today: a small win."));
        assert!(text.contains("What happened today: a small win."), "{text}");
        assert!(text.contains("who you are wins.)"), "{text}");
        assert_eq!(text.matches(NOTE_TAIL).count(), 1, "{text}");
    }

    #[test]
    fn before_and_after_noon_choose_yesterday_and_today() {
        let low = story::Levels {
            energy: Level::Low,
            stress: Level::High,
            mood: Level::High,
        };
        let high = story::Levels {
            energy: Level::High,
            stress: Level::Low,
            mood: Level::Low,
        };
        let today = story_day("2026-11-03", "today with {who}", low, high);
        let yesterday = story_day(
            "2026-11-02",
            "yesterday with {who}",
            story::Levels::default(),
            story::Levels::default(),
        );
        let names = BTreeMap::from([("theo".into(), "Theo".into())]);
        let morning = teller_lines(
            "2026-11-03T09:00:00".parse().unwrap(),
            &today,
            Some(&yesterday),
            &names,
        )
        .unwrap();
        assert!(morning.contains("What happened yesterday: yesterday with Theo."));
        assert!(morning.contains("low on energy, stressed and in a good mood"));
        assert!(morning.contains("rain, 4–9 °C (39–48 °F)"));
        assert!(!morning.contains("today with Theo"));

        let afternoon = teller_lines(
            "2026-11-03T12:00:00".parse().unwrap(),
            &today,
            Some(&yesterday),
            &names,
        )
        .unwrap();
        assert!(afternoon.contains("What happened today: today with Theo."));
        assert!(afternoon.contains("full of energy, relaxed and in a low mood"));
        assert!(!afternoon.contains("yesterday with Theo"));
    }

    #[test]
    fn a_story_error_keeps_the_doing_now_note() {
        let at = now(
            stretch(Activity::Reading, 10 * 60, 11 * 60),
            Source::Sampler,
        );
        let failed: anyhow::Result<String> = Err(anyhow::anyhow!("the deck does not load"));
        let text = note_with_events(&at, failed.as_ref().ok().map(String::as_str));
        assert!(text.contains("You are reading"), "{text}");
        assert!(!text.contains(EVENTS_HEADING), "{text}");
    }

    /// The fold is taught to all three predicates (§4.5): re-armed from the
    /// transcript and from a run's notes, left off the wire if it was ever
    /// stored, and never mined as the owner's words.
    #[test]
    fn both_stems_arm_on_resume_and_are_the_harness_speaking() {
        use crate::agent::Taint;
        use crate::message::{Block, Message};
        for (source, untrusted) in [(Source::Sampler, false), (Source::Unknown, true)] {
            let text = note(&now(stretch(Activity::Reading, 600, 660), source));
            let mut t = Taint::default();
            t.arm_for_content(&[Message::user(&text)]);
            assert_eq!(
                t,
                Taint {
                    private: true,
                    untrusted
                }
            );
            let mut n = Taint::default();
            n.arm_for_notes(&[String::from("a note first"), text.clone()]);
            assert_eq!(n, t);
            assert!(crate::message::is_recorded_note(&Block::text(text.clone())));
            assert!(crate::agent::is_harness_voice(&text));
        }
    }

    #[test]
    fn only_a_today_stem_is_one() {
        assert_eq!(
            stem_of("  (What you are doing right now, from the harness"),
            Some((true, false))
        );
        assert_eq!(stem_of("What you are doing right now"), None);
        assert_eq!(stem_of(crate::persona::recall::MEMORY_STEM), None);
    }

    #[test]
    fn the_teller_day_runs_four_to_four() {
        let at = |s: &str| -> NaiveDateTime { s.parse().unwrap() };
        assert!(before_noon(at("2026-11-03T09:00:00")));
        assert!(!before_noon(at("2026-11-03T13:00:00")));
        assert!(
            !before_noon(at("2026-11-04T01:00:00")),
            "1am is the evening before"
        );
        assert_eq!(
            diary_date(at("2026-11-04T01:00:00")).to_string(),
            "2026-11-03"
        );
        assert_eq!(
            diary_date(at("2026-11-04T04:00:00")).to_string(),
            "2026-11-04"
        );
    }
    /// An afternoon's teller, as [`teller`] writes it, with the weather
    /// line and sky a test names.
    fn told(weather: &str, sky: crate::persona::weather::Sky) -> Teller {
        let tired = story::Levels {
            energy: Level::Low,
            ..Default::default()
        };
        let d = story_day(
            "2026-03-03",
            "a flat tyre on the way to the market",
            story::Levels::default(),
            tired,
        );
        let at: NaiveDateTime = "2026-03-03T15:00:00".parse().unwrap();
        let mut t = teller(at, &d, None, &BTreeMap::new()).unwrap();
        assert!(t.feeling.is_some());
        t.weather = Some(weather.into());
        t.sky = Some(sky);
        t
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
    }

    /// The owner's report (2026-11-12): the day's lines rode every turn, and
    /// the weather came up in most replies of a chat that never asked.
    #[test]
    fn a_line_nobody_picked_up_stops_riding_after_its_offers() {
        use crate::persona::weather::Sky;
        let t = told("Today's weather is rain, 4–9 °C (39–48 °F).", Sky::Rain);
        let mut l = Ledger::default();
        for _ in 0..OFFERS {
            let lines = l
                .offer(day(3), &t, "what are you reading?", "A novel.")
                .unwrap();
            assert_eq!(lines, t.lines());
        }
        assert_eq!(l.offer(day(3), &t, "is it any good?", "It is."), None);
        assert_eq!(l.offer(day(3), &t, "who wrote it?", "No idea."), None);
    }

    #[test]
    fn a_line_she_said_is_dropped_and_the_others_keep_riding() {
        use crate::persona::weather::Sky;
        let t = told("Today's weather is rain, 4–9 °C (39–48 °F).", Sky::Rain);
        let mut l = Ledger::default();
        assert_eq!(l.offer(day(3), &t, "morning", ""), Some(t.lines()));
        // Her reply told the weather and nothing else.
        let next = l
            .offer(day(3), &t, "ha", "It's been raining since I got up.")
            .unwrap();
        assert_eq!(next, t.only(&[Item::Events, Item::Feeling]));
        // Then the tyre. The feeling has had its offers, so nothing rides.
        assert_eq!(
            l.offer(day(3), &t, "oh?", "I got a flat tyre, of all things."),
            None
        );
    }

    #[test]
    fn a_line_comes_back_on_the_turn_the_owner_asks_about_it() {
        use crate::persona::weather::Sky;
        let t = told("Today's weather is snow, -3–1 °C (27–34 °F).", Sky::Snow);
        let mut l = Ledger::default();
        for _ in 0..OFFERS {
            l.offer(day(3), &t, "hello", "Hello.");
        }
        assert_eq!(
            l.offer(
                day(3),
                &t,
                "which soup is it tonight, the leek one?",
                "Soup."
            ),
            None
        );
        assert_eq!(
            l.offer(day(3), &t, "is it freezing out there?", "Soup."),
            t.weather.clone()
        );
        assert_eq!(
            l.offer(day(3), &t, "and your day, did it go to plan?", "Freezing."),
            Some(t.events.clone())
        );
        assert_eq!(
            l.offer(
                day(3),
                &t,
                "are you feeling any better for it?",
                "Long one."
            ),
            t.feeling.clone()
        );
        assert_eq!(l.offer(day(3), &t, "and the book?", "Tired."), None);
    }

    /// A forecast refreshed mid-chat changed the line under her: one reply
    /// said snow and a later one drizzle (2026-11-12).
    #[test]
    fn the_weather_is_the_days_first_reading_and_a_new_day_starts_over() {
        use crate::persona::weather::Sky;
        let snow = told("Today's weather is snow, -3–1 °C (27–34 °F).", Sky::Snow);
        let drizzle = told(
            "Today's weather is drizzle, 2–6 °C (36–43 °F).",
            Sky::Drizzle,
        );
        let mut l = Ledger::default();
        l.offer(day(3), &snow, "hello", "");
        let asked = l
            .offer(day(3), &drizzle, "what's the weather doing?", "Snowing.")
            .unwrap();
        assert!(
            asked.contains("snow") && !asked.contains("drizzle"),
            "{asked}"
        );
        // The next diary day is a new day's lines, the new forecast with them.
        assert_eq!(
            l.offer(day(4), &drizzle, "morning", "Night."),
            Some(drizzle.lines())
        );
    }

    #[test]
    fn a_changed_events_line_is_a_new_line() {
        use crate::persona::weather::Sky;
        let morning = told("Today's weather is clear, 4–9 °C (39–48 °F).", Sky::Clear);
        let mut l = Ledger::default();
        for _ in 0..OFFERS {
            l.offer(day(3), &morning, "hello", "Hello.");
        }
        assert_eq!(l.offer(day(3), &morning, "hm", "Hm."), None);
        // Noon: the events line turns from yesterday's to today's.
        let mut noon = morning.clone();
        noon.events = format!("{EVENTS_HEADING} today: a long lunch with an old friend.");
        noon.event_words = content_words("a long lunch with an old friend");
        assert_eq!(
            l.offer(day(3), &noon, "hm", "Hm."),
            Some(noon.events.clone())
        );
    }

    #[test]
    fn everyday_words_are_not_the_weather_told() {
        use crate::persona::weather::Sky;
        let t = told("Today's weather is clear, 20–28 °C (68–82 °F).", Sky::Clear);
        let mut l = Ledger::default();
        l.offer(day(3), &t, "hello", "");
        let e = &l.entries[&Item::Weather];
        // "Sunday", "a hot coffee" and "clearly" say nothing of the sky.
        let reply = lowered("Clearly a hot coffee kind of Sunday.");
        assert!(!mentions(Item::Weather, e, &reply, false));
        assert!(mentions(Item::Weather, e, &lowered("So sunny out."), false));
        // Asked or not, an everyday word is not the weather; a curly
        // apostrophe is the plain one.
        assert!(!mentions(
            Item::Weather,
            e,
            &lowered("that is a hot take"),
            true
        ));
        assert!(mentions(
            Item::Weather,
            e,
            &lowered("what\u{2019}s the forecast?"),
            true
        ));
    }

    #[test]
    fn the_teller_joined_is_the_note_it_always_was() {
        let names = BTreeMap::new();
        let d = story_day(
            "2026-03-03",
            "a small win at work",
            story::Levels::default(),
            story::Levels::default(),
        );
        let at: NaiveDateTime = "2026-03-03T15:00:00".parse().unwrap();
        let t = teller(at, &d, None, &names).unwrap();
        assert_eq!(t.lines(), teller_lines(at, &d, None, &names).unwrap());
        assert_eq!(t.only(&[Item::Weather]), t.weather.clone().unwrap());
        assert!(
            t.event_words.contains(&"small".to_string()),
            "{:?}",
            t.event_words
        );
    }
}
