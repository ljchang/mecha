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
use super::world::Now;

/// The block's opening when every row in it is clean.
pub const TODAY_STEM: &str = "(What you are doing right now, from the harness";
/// The opening when any row in it came from outside.
pub const UNTRUSTED_TODAY_STEM: &str =
    "(What you are doing right now, some of it first read from outside, from the harness";

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

/// How the note names a kind of company.
fn who(c: Company) -> &'static str {
    match c {
        Company::Alone => "alone",
        Company::Partner => "your partner",
        Company::Child => "your child",
        Company::Parent => "a parent",
        Company::Family => "family",
        Company::Housemate => "a housemate",
        Company::Friend => "friends",
        Company::Acquaintance => "people you know",
        Company::Colleague => "colleagues",
        Company::Public => "people you don't know",
    }
}

/// The today-block for `now`. The times are the owner's clock, which is the
/// one her day runs on.
pub fn note(now: &Now) -> String {
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
            let names: Vec<&str> = kinds.iter().map(|k| who(*k)).collect();
            let listed = match names.split_last() {
                Some((last, [])) => last.to_string(),
                Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
                None => String::new(),
            };
            said.push_str(&format!(", with {listed}"));
        }
    }
    let stem = if now.source.is_clean() {
        TODAY_STEM
    } else {
        UNTRUSTED_TODAY_STEM
    };
    format!(
        "{stem} — your own day, drawn from your routine. {said}. Let it colour what you say \
         when it fits; it is never news to announce.)"
    )
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
        s.company = Some(vec![Company::Partner, Company::Family, Company::Colleague]);
        let text = note(&now(s, Source::Sampler));
        assert!(
            text.contains("with your partner, family and colleagues."),
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
