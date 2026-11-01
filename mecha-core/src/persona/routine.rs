//! Her week: `routine.toml` beside `persona.toml`
//! (`docs/PERSONA-LIFE-DESIGN.md` §4.1, ruled §16 PL1).
//!
//! The routine is the owner's file. Its anchors are things that happen at a
//! fixed time on given weekdays, each with a chance where real life is soft;
//! the day sampler fills the rest of the day from time-use data (§4.2). A
//! model may draft one into `routine.draft.toml`, beside the owner's and
//! never over it, and the draft becomes the routine only when the owner
//! approves it as shown. Nothing here reads a draft as a routine.
//!
//! A draft is staged only from a clean run. The anchors' labels are a
//! model's text once a draft is approved, so refusing an untrusted draft at
//! the door is what lets every routine the sampler reads be clean without
//! carrying an origin past approval.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::activity::Activity;
use super::Origin;

/// The routine the sampler reads.
pub const FILE: &str = "routine.toml";
/// A model's draft, waiting on the owner.
pub const DRAFT: &str = "routine.draft.toml";
/// The routine an approved draft replaced, kept so approval loses nothing.
pub const PREVIOUS: &str = "routine.previous.toml";
/// The digest of the draft as a model last staged it. A waiting draft that
/// no longer hashes to it was edited by the owner, and is theirs.
const STAGED: &str = ".routine.draft.staged";

/// More anchors than a week can sensibly hold is a draft gone wrong.
pub const MAX_ANCHORS: usize = 64;
/// An anchor's label is a few words for the week view.
pub const MAX_WHAT: usize = 80;
const MINUTES_PER_DAY: u32 = 24 * 60;
const MINUTES_PER_WEEK: u32 = 7 * MINUTES_PER_DAY;
const DAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// What a new `routine.toml` holds: every line a comment, so it loads as
/// a routine with no anchors and the sampler fills every hour.
pub const TEMPLATE: &str = "\
# Her week. Each [[anchor]] is something that happens at a fixed time;
# the rest of each day is filled in from time-use data.
#
# [[anchor]]
# what = \"gym\"             # a few words for the week view (optional)
# activity = \"exercise\"    # one of the activities `mecha persona routine show` lists
# days = [\"tue\", \"thu\"]    # mon..sun, or weekdays, weekends, daily
# at = \"07:00\"             # 24-hour clock
# minutes = 60
# chance = 0.7             # how often it really happens; 1.0 when left out
";

/// A validated routine.
#[derive(Debug, Clone, PartialEq)]
pub struct Routine {
    pub anchors: Vec<Anchor>,
}

/// One anchor, as the sampler places it.
#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    pub what: Option<String>,
    pub activity: Activity,
    /// Bit 0 is Monday, bit 6 Sunday.
    pub days: u8,
    /// Minutes after midnight.
    pub start: u32,
    pub minutes: u32,
    pub chance: f64,
}

impl Anchor {
    /// Whether it falls on `day` (0 = Monday).
    pub fn on(&self, day: u32) -> bool {
        day < 7 && self.days & (1 << day) != 0
    }

    /// How a refusal names it: its label, or its activity when it has none.
    fn name(&self) -> &str {
        self.what.as_deref().unwrap_or(self.activity.id())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileForm {
    #[serde(default)]
    anchor: Vec<AnchorForm>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorForm {
    what: Option<String>,
    activity: Activity,
    days: Vec<String>,
    at: String,
    minutes: u32,
    chance: Option<f64>,
}

/// Parse and check a routine's text. A refusal names the anchor by number
/// and label, and the value it refuses; nothing out of range is clamped.
pub fn parse(text: &str) -> Result<Routine> {
    // The store's own parser: a TOML escape cannot smuggle a control
    // character into a label that a refusal or `show` then prints.
    let form: FileForm = super::parse_toml(text)?;
    if form.anchor.len() > MAX_ANCHORS {
        bail!(
            "{} anchors; a routine holds at most {MAX_ANCHORS}",
            form.anchor.len()
        );
    }
    let mut anchors = Vec::with_capacity(form.anchor.len());
    for (i, a) in form.anchor.into_iter().enumerate() {
        let n = i + 1;
        let label = a.what.clone().unwrap_or_else(|| a.activity.id().into());
        let refuse = |why: String| anyhow::anyhow!("anchor {n} ({label}): {why}");
        let what = match a.what {
            None => None,
            Some(w) if w.trim().is_empty() => {
                return Err(refuse("`what` is empty; leave it out instead".into()))
            }
            Some(w) if w.chars().count() > MAX_WHAT => {
                return Err(refuse(format!(
                    "`what` is {} characters; at most {MAX_WHAT}",
                    w.chars().count()
                )))
            }
            Some(w) if w.chars().any(char::is_control) => {
                return Err(refuse("`what` holds a control character".into()))
            }
            Some(w) => Some(w.trim().to_string()),
        };
        let days = days_of(&a.days).map_err(refuse)?;
        let start = time_of(&a.at).ok_or_else(|| {
            refuse(format!(
                "at = {:?} is not a time: write it as HH:MM, like \"07:00\"",
                a.at
            ))
        })?;
        if !(5..=MINUTES_PER_DAY).contains(&a.minutes) {
            return Err(refuse(format!(
                "minutes = {} is out of range: it must be 5 to {MINUTES_PER_DAY}",
                a.minutes
            )));
        }
        let chance = a.chance.unwrap_or(1.0);
        if !(chance.is_finite() && chance > 0.0 && chance <= 1.0) {
            return Err(refuse(format!(
                "chance = {chance} is out of range: it must be above 0 and at most 1"
            )));
        }
        anchors.push(Anchor {
            what,
            activity: a.activity,
            days,
            start,
            minutes: a.minutes,
            chance,
        });
    }
    certain_clashes(&anchors)?;
    Ok(Routine { anchors })
}

fn days_of(words: &[String]) -> std::result::Result<u8, String> {
    let mut mask = 0u8;
    for w in words {
        let w = w.trim().to_lowercase();
        mask |= match w.as_str() {
            "weekdays" => 0b001_1111,
            "weekends" => 0b110_0000,
            "daily" => 0b111_1111,
            _ => match DAY_NAMES.iter().position(|d| *d == w) {
                Some(i) => 1 << i,
                None => {
                    return Err(format!(
                        "days names {w:?}; use mon..sun, weekdays, weekends or daily"
                    ))
                }
            },
        };
    }
    if mask == 0 {
        return Err("days is empty; name at least one".into());
    }
    Ok(mask)
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

/// Two anchors that always happen cannot overlap: she cannot be at both.
/// Soft anchors may, and the sampler settles which one a day keeps. Times
/// are laid on the week as a circle, so Sunday night runs into Monday.
fn certain_clashes(anchors: &[Anchor]) -> Result<()> {
    let spans = |a: &Anchor| -> Vec<(u32, u32)> {
        (0..7)
            .filter(|d| a.on(*d))
            .map(|d| {
                let s = d * MINUTES_PER_DAY + a.start;
                (s, s + a.minutes)
            })
            .collect()
    };
    for (i, a) in anchors.iter().enumerate() {
        for (j, b) in anchors.iter().enumerate().skip(i + 1) {
            if a.chance < 1.0 || b.chance < 1.0 {
                continue;
            }
            for (s1, e1) in spans(a) {
                for (s2, e2) in spans(b) {
                    // Each span against the other as laid, and a week later.
                    let clash = [0, MINUTES_PER_WEEK].iter().find_map(|shift| {
                        if s1 < e2 + shift && s2 + shift < e1 {
                            Some(s1.max(s2 + shift))
                        } else if s2 < e1 + shift && s1 + shift < e2 {
                            Some(s2.max(s1 + shift))
                        } else {
                            None
                        }
                    });
                    if let Some(from) = clash {
                        bail!(
                            "anchors {} ({}) and {} ({}) both always happen and overlap on {}; \
                             move one, or give one a chance below 1",
                            i + 1,
                            a.name(),
                            j + 1,
                            b.name(),
                            DAY_NAMES[((from / MINUTES_PER_DAY) % 7) as usize],
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn persona_dir(dir: &Path, name: &str) -> Result<PathBuf> {
    super::validate_persona_name(name)?;
    let pdir = dir.join(name);
    if !pdir.join("persona.toml").is_file() {
        bail!("no persona named `{name}`");
    }
    Ok(pdir)
}

fn read_if_there(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    super::read_prose(path).map(Some)
}

/// The routine the sampler would read: `None` when the persona has none.
/// A file that does not parse is an error naming it, never an empty week.
pub fn load(pdir: &Path) -> Result<Option<Routine>> {
    let Some(text) = read_if_there(&pdir.join(FILE))? else {
        return Ok(None);
    };
    parse(&text)
        .map(Some)
        .with_context(|| format!("{FILE} does not load"))
}

/// `routine.toml`'s path, written from [`TEMPLATE`] first if the persona
/// has none, owner-only like the rest of the store.
pub fn ensure(dir: &Path, name: &str) -> Result<PathBuf> {
    let path = persona_dir(dir, name)?.join(FILE);
    super::write_new(&path, TEMPLATE)?;
    Ok(path)
}

/// The routine's text and a waiting draft's, as they stand.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Texts {
    pub routine: Option<String>,
    pub draft: Option<String>,
}

pub fn read(dir: &Path, name: &str) -> Result<Texts> {
    let pdir = persona_dir(dir, name)?;
    Ok(Texts {
        routine: read_if_there(&pdir.join(FILE))?,
        draft: read_if_there(&pdir.join(DRAFT))?,
    })
}

/// Stage a model's draft beside the owner's routine, replacing a draft
/// still waiting (revision until approval) and never the routine itself.
/// Refused unless it parses and holds an anchor, unless the run that wrote
/// it was clean, and over a waiting draft the owner has edited: a model
/// never overwrites the owner's text (§4.1).
pub fn stage_draft(dir: &Path, name: &str, text: &str, origin: Origin) -> Result<()> {
    match origin {
        Origin::Owner => bail!("a draft is a model's; the owner edits {FILE} directly"),
        Origin::ModelUntrusted => bail!(
            "a routine draft from a run that read untrusted content is refused; \
             once approved its labels would reach her days as clean"
        ),
        Origin::ModelClean => {}
    }
    let pdir = persona_dir(dir, name)?;
    if text.len() as u64 > super::MAX_PROSE_BYTES {
        bail!(
            "the draft is capped at {} KB",
            super::MAX_PROSE_BYTES / 1024
        );
    }
    if let Some(c) = super::forbidden_control(text) {
        bail!("the draft holds a control character (U+{:04X})", c as u32);
    }
    let routine = parse(text).context("the draft does not load")?;
    if routine.anchors.is_empty() {
        bail!("the draft holds no anchors");
    }
    if let Some(waiting) = read_if_there(&pdir.join(DRAFT))? {
        // No record, or one that disagrees, is an edit: unknown is the
        // owner's here, because the cost of guessing wrong is their work.
        let staged = std::fs::read_to_string(pdir.join(STAGED)).unwrap_or_default();
        if staged.trim() != super::text_digest(&waiting) {
            bail!(
                "`{name}`'s waiting routine draft has been edited since it was staged; \
                 approve or discard it before another is drafted"
            );
        }
    }
    super::write_private(&pdir.join(DRAFT), text.as_bytes())?;
    super::write_private(&pdir.join(STAGED), super::text_digest(text).as_bytes())
}

/// Make the waiting draft the routine, only if it still hashes to
/// `digest`, what the owner was shown. The routine it replaces is kept as
/// [`PREVIOUS`].
pub fn approve_draft_as_shown(dir: &Path, name: &str, digest: &str) -> Result<Routine> {
    let pdir = persona_dir(dir, name)?;
    let Some(text) = read_if_there(&pdir.join(DRAFT))? else {
        bail!("`{name}` has no routine draft waiting");
    };
    if super::text_digest(&text) != digest {
        bail!(
            "`{name}`'s routine draft changed since it was shown — read it again before approving"
        );
    }
    // The owner may have edited the draft into something that no longer
    // loads; approving that would leave her with no week at all.
    let routine = parse(&text).context("the draft does not load")?;
    if let Some(old) = read_if_there(&pdir.join(FILE))? {
        super::write_private(&pdir.join(PREVIOUS), old.as_bytes())?;
    }
    super::write_private(&pdir.join(FILE), text.as_bytes())?;
    std::fs::remove_file(pdir.join(DRAFT))
        .with_context(|| format!("the routine is approved, but {DRAFT} could not be removed"))?;
    let _ = std::fs::remove_file(pdir.join(STAGED));
    Ok(routine)
}

/// Drop a waiting draft. The routine is untouched.
pub fn discard_draft(dir: &Path, name: &str) -> Result<()> {
    let pdir = persona_dir(dir, name)?;
    match std::fs::remove_file(pdir.join(DRAFT)) {
        Ok(()) => {
            let _ = std::fs::remove_file(pdir.join(STAGED));
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("`{name}` has no routine draft waiting")
        }
        Err(e) => Err(e).with_context(|| format!("removing {DRAFT}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::tests_support::*;

    const GYM: &str = r#"
[[anchor]]
what = "gym"
activity = "exercise"
days = ["tue", "thu"]
at = "07:00"
minutes = 60
chance = 0.7
"#;

    fn made() -> PathBuf {
        let dir = scratch();
        crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        dir
    }

    fn refusal(text: &str) -> String {
        format!("{:#}", parse(text).unwrap_err())
    }

    #[test]
    fn an_anchor_reads_as_written() {
        let r = parse(GYM).unwrap();
        assert_eq!(
            r.anchors,
            vec![Anchor {
                what: Some("gym".into()),
                activity: Activity::Exercise,
                days: 0b000_1010,
                start: 7 * 60,
                minutes: 60,
                chance: 0.7,
            }]
        );
        assert!(r.anchors[0].on(1) && r.anchors[0].on(3) && !r.anchors[0].on(0));
    }

    #[test]
    fn the_template_loads_as_a_week_with_no_anchors() {
        assert_eq!(parse(TEMPLATE).unwrap().anchors, vec![]);
    }

    #[test]
    fn day_words_expand_and_chance_defaults_to_always() {
        let r = parse(
            "[[anchor]]\nactivity = \"work\"\ndays = [\"weekdays\"]\nat = \"9:00\"\nminutes = 480\n\
             [[anchor]]\nactivity = \"religion\"\ndays = [\"sun\", \"weekends\"]\nat = \"10:00\"\nminutes = 90\n",
        )
        .unwrap();
        assert_eq!(r.anchors[0].days, 0b001_1111);
        assert_eq!(r.anchors[0].chance, 1.0);
        assert_eq!(r.anchors[1].days, 0b110_0000);
    }

    #[test]
    fn each_bad_value_is_refused_by_anchor_and_name_never_clamped() {
        for (from, to, want) in [
            (
                "at = \"07:00\"",
                "at = \"7am\"",
                "anchor 1 (gym): at = \"7am\" is not a time",
            ),
            ("at = \"07:00\"", "at = \"24:00\"", "not a time"),
            ("at = \"07:00\"", "at = \"+7:00\"", "not a time"),
            ("minutes = 60", "minutes = 2", "minutes = 2 is out of range"),
            (
                "minutes = 60",
                "minutes = 1441",
                "minutes = 1441 is out of range",
            ),
            ("chance = 0.7", "chance = 0.0", "chance = 0 is out of range"),
            (
                "chance = 0.7",
                "chance = 1.5",
                "chance = 1.5 is out of range",
            ),
            ("[\"tue\", \"thu\"]", "[\"tues\"]", "days names \"tues\""),
            ("[\"tue\", \"thu\"]", "[]", "days is empty"),
            ("what = \"gym\"", "what = \" \"", "`what` is empty"),
            ("\"exercise\"", "\"jogging\"", "jogging"),
            ("chance = 0.7", "chance = 0.7\nwhere = \"park\"", "where"),
        ] {
            assert!(GYM.contains(from), "{from}");
            let got = refusal(&GYM.replace(from, to));
            assert!(got.contains(want), "{to}: {got}");
        }
    }

    #[test]
    fn an_escaped_control_character_never_reaches_a_label() {
        let got = refusal(&GYM.replace("what = \"gym\"", "what = \"gym\\u001b[2J\""));
        assert!(got.contains("control character (U+001B)"), "{got}");
        assert!(
            !got.contains('\u{1b}'),
            "the refusal carries the escape itself"
        );
    }

    #[test]
    fn two_certain_anchors_cannot_overlap_and_soft_ones_may() {
        let two = |chance: &str| {
            format!(
                "[[anchor]]\nwhat = \"shift\"\nactivity = \"work\"\ndays = [\"sun\"]\nat = \"22:00\"\nminutes = 480\n\
                 [[anchor]]\nwhat = \"swim\"\nactivity = \"exercise\"\ndays = [\"mon\"]\nat = \"05:00\"\nminutes = 60\n{chance}"
            )
        };
        // Sunday's night shift runs past midnight into Monday's early swim.
        let got = format!("{:#}", parse(&two("")).unwrap_err());
        assert!(
            got.contains("anchors 1 (shift) and 2 (swim) both always happen and overlap on mon"),
            "{got}"
        );
        parse(&two("chance = 0.9")).unwrap();
        // Back to back is not an overlap.
        parse(&two("").replace("at = \"05:00\"", "at = \"06:00\"")).unwrap();
    }

    #[test]
    fn a_draft_waits_beside_the_routine_and_approval_swaps_them() {
        let dir = made();
        let pdir = dir.join("mara");
        std::fs::write(pdir.join(FILE), TEMPLATE).unwrap();

        stage_draft(&dir, "mara", GYM, Origin::ModelClean).unwrap();
        let texts = read(&dir, "mara").unwrap();
        assert_eq!(
            texts.routine.as_deref(),
            Some(TEMPLATE),
            "the draft went over the routine"
        );
        assert_eq!(texts.draft.as_deref(), Some(GYM));
        // The sampler's door never reads the draft.
        assert_eq!(load(&pdir).unwrap().unwrap().anchors, vec![]);

        let stale = approve_draft_as_shown(&dir, "mara", &crate::persona::text_digest("other"));
        assert!(format!("{:#}", stale.unwrap_err()).contains("changed since it was shown"));

        approve_draft_as_shown(&dir, "mara", &crate::persona::text_digest(GYM)).unwrap();
        assert_eq!(load(&pdir).unwrap().unwrap().anchors.len(), 1);
        assert!(!pdir.join(DRAFT).exists());
        assert_eq!(
            std::fs::read_to_string(pdir.join(PREVIOUS)).unwrap(),
            TEMPLATE
        );
    }

    #[test]
    fn a_draft_is_a_clean_models_and_must_load_with_an_anchor() {
        let dir = made();
        for (text, origin, want) in [
            (GYM, Origin::Owner, "a draft is a model's"),
            (GYM, Origin::ModelUntrusted, "untrusted content is refused"),
            (TEMPLATE, Origin::ModelClean, "holds no anchors"),
            (
                "[[anchor]]\nactivity = \"nap\"",
                Origin::ModelClean,
                "does not load",
            ),
        ] {
            let got = format!("{:#}", stage_draft(&dir, "mara", text, origin).unwrap_err());
            assert!(got.contains(want), "{got}");
        }
        assert_eq!(read(&dir, "mara").unwrap(), Texts::default());
    }

    #[test]
    fn a_routine_that_does_not_parse_is_an_error_never_an_empty_week() {
        let dir = made();
        let pdir = dir.join("mara");
        assert_eq!(load(&pdir).unwrap(), None);
        std::fs::write(pdir.join(FILE), GYM.replace("07:00", "7am")).unwrap();
        let got = format!("{:#}", load(&pdir).unwrap_err());
        assert!(
            got.contains("routine.toml does not load") && got.contains("7am"),
            "{got}"
        );
    }

    #[test]
    fn a_model_never_restages_over_a_draft_the_owner_edited() {
        let dir = made();
        let draft = dir.join("mara").join(DRAFT);
        stage_draft(&dir, "mara", GYM, Origin::ModelClean).unwrap();
        // Revising its own waiting draft is allowed.
        let revised = GYM.replace("0.7", "0.6");
        stage_draft(&dir, "mara", &revised, Origin::ModelClean).unwrap();
        std::fs::write(&draft, revised.replace("0.6", "0.9")).unwrap();
        let got = format!(
            "{:#}",
            stage_draft(&dir, "mara", GYM, Origin::ModelClean).unwrap_err()
        );
        assert!(got.contains("edited since it was staged"), "{got}");
        assert!(std::fs::read_to_string(&draft).unwrap().contains("0.9"));
        // Once it is dealt with, a new draft may be staged.
        discard_draft(&dir, "mara").unwrap();
        stage_draft(&dir, "mara", GYM, Origin::ModelClean).unwrap();
    }

    #[test]
    fn discarding_drops_only_the_draft() {
        let dir = made();
        std::fs::write(dir.join("mara").join(FILE), GYM).unwrap();
        assert!(
            format!("{:#}", discard_draft(&dir, "mara").unwrap_err()).contains("no routine draft")
        );
        stage_draft(&dir, "mara", GYM, Origin::ModelClean).unwrap();
        discard_draft(&dir, "mara").unwrap();
        let texts = read(&dir, "mara").unwrap();
        assert_eq!((texts.routine.as_deref(), texts.draft), (Some(GYM), None));
    }
}
