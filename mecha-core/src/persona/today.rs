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
    at.hour() < 12
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
    let mut lines = vec![format!(
        "{EVENTS_HEADING} {}: {happened}.",
        if morning { "yesterday" } else { "today" }
    )];

    let levels = if morning {
        today.morning
    } else {
        today.evening
    };
    let mut feeling = Vec::new();
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
        lines.push(format!("You feel {said}."));
    }
    if let Some(w) = today.weather {
        lines.push(format!(
            "Today's weather is {}, {}–{} °C ({}–{} °F).",
            w.sky.id(),
            w.low_c,
            w.high_c,
            fahrenheit(w.low_c),
            fahrenheit(w.high_c)
        ));
    }
    Ok(lines.join("\n"))
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
}
