//! The owner's profile of the life a sampled day may hold: `life.toml`
//! beside `routine.toml` (`docs/PERSONA-LIFE-DESIGN.md` §4.2).
//!
//! A profile is opt-in. Without this file the sampler reads the time-use
//! table exactly as it did before. With one, household company must name a
//! relation the owner put here, forbidden activities are removed from the
//! chain, and an optional usual night constrains its sleep. This is owner
//! text and therefore clean; code writes only the commented starter once.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::activity::Activity;
use super::atus::Company;

pub const FILE: &str = "life.toml";
pub const MAX_HOUSEHOLD: usize = 12;

/// A commented starter is an empty profile: present, but making no claims
/// about a household or usual night.
pub const TEMPLATE: &str = "\
# Their life: who they live with and what their days can hold.
#
# alone = false
# never = [\"caring\"]       # activity ids their days never hold; not sleep
#
# [[household]]
# person = \"Theo\"          # a name on their People tab
# relation = \"housemate\"   # partner, child, parent, family or housemate
#
# [sleep]
# bed = \"23:30\"            # their usual night, 24-hour clock
# wake = \"07:30\"
";

#[derive(Debug, Clone, PartialEq)]
pub struct Life {
    pub alone: bool,
    pub household: Vec<Household>,
    pub sleep: Option<Sleep>,
    pub never: Vec<Activity>,
}

impl Life {
    pub fn has(&self, relation: Relation) -> bool {
        !self.alone && self.household.iter().any(|h| h.relation == relation)
    }

    pub fn keeps(&self, company: Company) -> bool {
        Relation::from_company(company).is_none_or(|r| self.has(r))
    }

    pub fn allows(&self, activity: Activity) -> bool {
        if self.never.contains(&activity) {
            return false;
        }
        activity != Activity::Caring
            || [Relation::Child, Relation::Parent, Relation::Partner]
                .into_iter()
                .any(|r| self.has(r))
    }

    pub fn names_for(&self, kinds: &[Company]) -> Vec<String> {
        let mut names = Vec::new();
        for h in &self.household {
            if kinds.contains(&h.relation.company())
                && !names
                    .iter()
                    .any(|n: &String| n.eq_ignore_ascii_case(&h.person))
            {
                names.push(h.person.clone());
            }
        }
        names
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "household": self.household.iter().map(|h| serde_json::json!({
                "person": h.person.as_str(),
                "relation": h.relation.id(),
            })).collect::<Vec<_>>(),
            "alone": self.alone,
            "sleep": self.sleep.as_ref().map(|s| serde_json::json!({
                "bed": clock(s.bed),
                "wake": clock(s.wake),
            })),
            "never": self.never.iter().map(|a| a.id()).collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Household {
    pub person: String,
    pub relation: Relation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Partner,
    Child,
    Parent,
    Family,
    Housemate,
}

impl Relation {
    pub const ALL: [Relation; 5] = [
        Relation::Partner,
        Relation::Child,
        Relation::Parent,
        Relation::Family,
        Relation::Housemate,
    ];

    pub fn id(self) -> &'static str {
        self.company().id()
    }

    pub fn company(self) -> Company {
        match self {
            Relation::Partner => Company::Partner,
            Relation::Child => Company::Child,
            Relation::Parent => Company::Parent,
            Relation::Family => Company::Family,
            Relation::Housemate => Company::Housemate,
        }
    }

    pub fn from_id(id: &str) -> Option<Relation> {
        Relation::ALL.into_iter().find(|r| r.id() == id)
    }

    pub fn from_company(company: Company) -> Option<Relation> {
        Relation::ALL.into_iter().find(|r| r.company() == company)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sleep {
    /// Minutes after midnight.
    pub bed: u32,
    pub wake: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileForm {
    #[serde(default)]
    alone: bool,
    #[serde(default)]
    household: Vec<HouseholdForm>,
    sleep: Option<SleepForm>,
    #[serde(default)]
    never: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HouseholdForm {
    person: String,
    relation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SleepForm {
    bed: String,
    wake: String,
}

pub fn parse(text: &str) -> Result<Life> {
    let form: FileForm = super::parse_toml(text)?;
    if form.household.len() > MAX_HOUSEHOLD {
        bail!(
            "{} household entries; a life profile holds at most {MAX_HOUSEHOLD}",
            form.household.len()
        );
    }
    if form.alone && !form.household.is_empty() {
        bail!(
            "alone = true disagrees with {} household entry(s); remove one or the other",
            form.household.len()
        );
    }
    let mut household = Vec::with_capacity(form.household.len());
    for (i, h) in form.household.into_iter().enumerate() {
        let person = h.person.trim();
        if person.is_empty() {
            bail!("household entry {}: person is empty", i + 1);
        }
        let relation = Relation::from_id(h.relation.trim()).ok_or_else(|| {
            anyhow::anyhow!(
                "household entry {} ({person}): relation = {:?} is not one of {}",
                i + 1,
                h.relation,
                Relation::ALL.map(Relation::id).join(", ")
            )
        })?;
        household.push(Household {
            person: person.to_string(),
            relation,
        });
    }
    let sleep = form
        .sleep
        .map(|s| {
            let bed = time_of(&s.bed).ok_or_else(|| {
                anyhow::anyhow!(
                    "sleep bed = {:?} is not a time: write it as HH:MM, like \"23:30\"",
                    s.bed
                )
            })?;
            let wake = time_of(&s.wake).ok_or_else(|| {
                anyhow::anyhow!(
                    "sleep wake = {:?} is not a time: write it as HH:MM, like \"07:30\"",
                    s.wake
                )
            })?;
            if bed == wake {
                bail!(
                    "sleep bed = {:?} and wake = {:?} are the same time; a usual sleep needs a start and an end",
                    s.bed,
                    s.wake
                );
            }
            Ok::<_, anyhow::Error>(Sleep { bed, wake })
        })
        .transpose()?;
    let mut never = Vec::new();
    for word in form.never {
        let id = word.trim();
        let activity = Activity::from_id(id)
            .ok_or_else(|| anyhow::anyhow!("never names unknown activity {word:?}"))?;
        if activity == Activity::Sleep {
            bail!("never contains \"sleep\"; a sampled day must be able to sleep");
        }
        if !never.contains(&activity) {
            never.push(activity);
        }
    }
    Ok(Life {
        alone: form.alone,
        household,
        sleep,
        never,
    })
}

fn time_of(at: &str) -> Option<u32> {
    let (h, m) = at.trim().split_once(':')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(h) || h.len() > 2 || !digits(m) || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

fn clock(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn persona_dir(dir: &Path, name: &str) -> Result<PathBuf> {
    super::validate_persona_name(name)?;
    let pdir = dir.join(name);
    if !pdir.join("persona.toml").is_file() {
        bail!("no persona named `{name}`");
    }
    Ok(pdir)
}

/// The profile the sampler reads. A missing file is the exact unprofiled
/// behaviour; a bad one is an error naming `life.toml`, never ignored.
pub fn load(pdir: &Path) -> Result<Option<Life>> {
    let path = pdir.join(FILE);
    if !path.exists() {
        return Ok(None);
    }
    let text = super::read_prose(&path)?;
    parse(&text)
        .map(Some)
        .with_context(|| format!("{FILE} does not load"))
}

pub fn ensure(dir: &Path, name: &str) -> Result<PathBuf> {
    let path = persona_dir(dir, name)?.join(FILE);
    super::write_new(&path, TEMPLATE)?;
    Ok(path)
}

/// The file as the owner wrote it, or `None` when they have not opted in.
pub fn read(dir: &Path, name: &str) -> Result<Option<String>> {
    let path = persona_dir(dir, name)?.join(FILE);
    if !path.exists() {
        return Ok(None);
    }
    super::read_prose(&path).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(text: &str) -> String {
        format!("{:#}", parse(text).unwrap_err())
    }

    #[test]
    fn a_profile_reads_as_written_and_the_template_is_empty() {
        let life = parse(
            "alone = false\nnever = [\"work\", \"caring\"]\n\
             [[household]]\nperson = \"Theo\"\nrelation = \"housemate\"\n\
             [sleep]\nbed = \"23:30\"\nwake = \"07:30\"\n",
        )
        .unwrap();
        assert_eq!(
            life,
            Life {
                alone: false,
                household: vec![Household {
                    person: "Theo".into(),
                    relation: Relation::Housemate,
                }],
                sleep: Some(Sleep {
                    bed: 23 * 60 + 30,
                    wake: 7 * 60 + 30,
                }),
                never: vec![Activity::Work, Activity::Caring],
            }
        );
        assert_eq!(
            parse(TEMPLATE).unwrap(),
            Life {
                alone: false,
                household: vec![],
                sleep: None,
                never: vec![],
            }
        );
    }

    #[test]
    fn every_bad_value_is_refused_by_name() {
        let too_many = (0..=MAX_HOUSEHOLD)
            .map(|i| format!("[[household]]\nperson = \"Pell {i}\"\nrelation = \"family\"\n"))
            .collect::<String>();
        for (text, named) in [
            (
                "alone = true\n[[household]]\nperson = \"Theo\"\nrelation = \"housemate\"\n",
                "alone = true",
            ),
            (
                "[[household]]\nperson = \"Theo\"\nrelation = \"neighbour\"\n",
                "neighbour",
            ),
            (
                "[[household]]\nperson = \" \"\nrelation = \"family\"\n",
                "person is empty",
            ),
            ("never = [\"juggling\"]\n", "juggling"),
            ("never = [\"sleep\"]\n", "sleep"),
            ("[sleep]\nbed = \"25:00\"\nwake = \"07:00\"\n", "25:00"),
            ("[sleep]\nbed = \"23:00\"\nwake = \"noon\"\n", "noon"),
            (
                "[sleep]\nbed = \"08:00\"\nwake = \"08:00\"\n",
                "the same time",
            ),
            (&too_many, "13 household entries"),
        ] {
            let why = refusal(text);
            assert!(why.contains(named), "{named:?} not in {why:?}");
        }
        assert!(refusal("surprise = true\n").contains("surprise"));
    }
}
