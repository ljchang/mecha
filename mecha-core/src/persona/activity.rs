//! What a persona can be doing: the closed activity set her world runs on
//! (`docs/PERSONA-LIFE-DESIGN.md` §4.2, §4.5).
//!
//! One set serves the routine's anchors, the day sampler's chain and a typed
//! plan, so a plan can only name what a day can hold. It is fixed in code
//! and capped at 32 entries, because a typed plan spends five bits on it
//! (§16 PL21). Each activity owns a few ATUS lexicon code prefixes, and the
//! longest prefix that matches a diary code wins, so `120303` (television)
//! is [`Activity::Tv`] while the rest of `12` is [`Activity::Relaxing`].

use serde::{Deserialize, Serialize};

/// An activity, as the routine names it and the sampler draws it.
///
/// Owner-written files refuse a value not listed here; a store that keeps
/// activities is a wire format and narrows an unknown one itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Sleep,
    PersonalCare,
    Eating,
    Cooking,
    Housework,
    Shopping,
    Errands,
    Caring,
    Work,
    Class,
    Studying,
    Commute,
    Travel,
    Socializing,
    Tv,
    Music,
    Games,
    Computer,
    Hobbies,
    Reading,
    Relaxing,
    Outing,
    Exercise,
    SportsEvent,
    Religion,
    Volunteering,
}

impl Activity {
    pub const ALL: [Activity; 26] = [
        Activity::Sleep,
        Activity::PersonalCare,
        Activity::Eating,
        Activity::Cooking,
        Activity::Housework,
        Activity::Shopping,
        Activity::Errands,
        Activity::Caring,
        Activity::Work,
        Activity::Class,
        Activity::Studying,
        Activity::Commute,
        Activity::Travel,
        Activity::Socializing,
        Activity::Tv,
        Activity::Music,
        Activity::Games,
        Activity::Computer,
        Activity::Hobbies,
        Activity::Reading,
        Activity::Relaxing,
        Activity::Outing,
        Activity::Exercise,
        Activity::SportsEvent,
        Activity::Religion,
        Activity::Volunteering,
    ];

    /// The name a file writes, and the one a refusal quotes.
    pub fn id(self) -> &'static str {
        match self {
            Activity::Sleep => "sleep",
            Activity::PersonalCare => "personal_care",
            Activity::Eating => "eating",
            Activity::Cooking => "cooking",
            Activity::Housework => "housework",
            Activity::Shopping => "shopping",
            Activity::Errands => "errands",
            Activity::Caring => "caring",
            Activity::Work => "work",
            Activity::Class => "class",
            Activity::Studying => "studying",
            Activity::Commute => "commute",
            Activity::Travel => "travel",
            Activity::Socializing => "socializing",
            Activity::Tv => "tv",
            Activity::Music => "music",
            Activity::Games => "games",
            Activity::Computer => "computer",
            Activity::Hobbies => "hobbies",
            Activity::Reading => "reading",
            Activity::Relaxing => "relaxing",
            Activity::Outing => "outing",
            Activity::Exercise => "exercise",
            Activity::SportsEvent => "sports_event",
            Activity::Religion => "religion",
            Activity::Volunteering => "volunteering",
        }
    }

    /// The ATUS lexicon code prefixes this activity owns. Tier-1 is two
    /// digits, tier-2 four, tier-3 six; `50` (data codes) is nobody's.
    pub fn atus(self) -> &'static [&'static str] {
        match self {
            Activity::Sleep => &["0101"],
            Activity::PersonalCare => &["01"],
            Activity::Eating => &["11"],
            Activity::Cooking => &["0202"],
            Activity::Housework => &["02"],
            Activity::Shopping => &["07"],
            Activity::Errands => &["08", "09", "10"],
            Activity::Caring => &["03", "04"],
            Activity::Work => &["05"],
            Activity::Class => &["06"],
            Activity::Studying => &["0603"],
            Activity::Commute => &["1805", "1806"],
            Activity::Travel => &["18"],
            Activity::Socializing => &["1201", "1202", "16"],
            Activity::Tv => &["120303", "120304"],
            Activity::Music => &["120305", "120306"],
            Activity::Games => &["120307"],
            Activity::Computer => &["120308"],
            Activity::Hobbies => &["120309", "120310", "120311", "120313"],
            Activity::Reading => &["120312"],
            Activity::Relaxing => &["12"],
            Activity::Outing => &["1204"],
            Activity::Exercise => &["13"],
            Activity::SportsEvent => &["1302"],
            Activity::Religion => &["14"],
            Activity::Volunteering => &["15"],
        }
    }

    /// The activity an ATUS six-digit code belongs to: the longest prefix
    /// any activity owns. `None` for a code no activity claims.
    pub fn of_atus(code: &str) -> Option<Activity> {
        Activity::ALL
            .iter()
            .flat_map(|a| a.atus().iter().map(move |p| (*a, *p)))
            .filter(|(_, p)| code.starts_with(p))
            .max_by_key(|(_, p)| p.len())
            .map(|(a, _)| a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_set_fits_the_five_bits_a_plan_spends_on_it() {
        assert!(Activity::ALL.len() <= 32);
        let ids: BTreeSet<_> = Activity::ALL.iter().map(|a| a.id()).collect();
        assert_eq!(ids.len(), Activity::ALL.len(), "two activities share an id");
        for a in Activity::ALL {
            // The id is the serde name, so a file and a refusal agree.
            assert_eq!(serde_json::to_value(a).unwrap(), a.id());
        }
    }

    #[test]
    fn no_prefix_is_owned_twice_so_a_code_has_one_activity() {
        let mut seen = BTreeSet::new();
        for a in Activity::ALL {
            for p in a.atus() {
                assert!(seen.insert(*p), "ATUS prefix {p} is owned twice");
                assert!(matches!(p.len(), 2 | 4 | 6), "{p} is not a lexicon tier");
            }
        }
    }

    #[test]
    fn every_tier_one_category_lands_somewhere() {
        // 17 is unused in the lexicon, and 50 holds data codes.
        for major in (1..=18).filter(|m| *m != 17) {
            let code = format!("{major:02}9999");
            assert!(Activity::of_atus(&code).is_some(), "{code} has no activity");
        }
        assert_eq!(Activity::of_atus("500101"), None);
    }

    #[test]
    fn the_longest_prefix_wins() {
        for (code, want) in [
            ("010101", Activity::Sleep),
            ("010201", Activity::PersonalCare),
            ("020201", Activity::Cooking),
            ("020101", Activity::Housework),
            ("060301", Activity::Studying),
            ("060101", Activity::Class),
            ("180501", Activity::Commute),
            ("180701", Activity::Travel),
            ("120303", Activity::Tv),
            ("120312", Activity::Reading),
            ("120301", Activity::Relaxing),
            ("129999", Activity::Relaxing),
            ("130201", Activity::SportsEvent),
            ("130101", Activity::Exercise),
            ("160101", Activity::Socializing),
        ] {
            assert_eq!(Activity::of_atus(code), Some(want), "{code}");
        }
    }
}
