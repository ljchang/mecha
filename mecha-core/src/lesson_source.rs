//! The reflector's lessons against the appraisal's, on the same
//! interventions (`docs/APPRAISAL-WIRING-DESIGN.md` row 2e-1, L2, R25).
//!
//! R25 lets the reflector fold into the appraisal (row 2a-4) only after its
//! lessons measure no worse than the appraisal's. This is that measurement,
//! and it is **shadow**: a lesson from either source reaches one probe arm's
//! system prompt and nothing else — never the rules store, never a run.
//!
//! **The unit is an intervention the reflector already reflected on** — a
//! steer or a denial, the two triggers a structural validator grades (R27;
//! a followup is judge-graded and is excluded, counted). At each one three
//! arms are driven by the validation probe (`probe::drive_arm`, the one
//! `mecha validate` and the learn gate use): the recorded prompt with its
//! rules removed, and the same with one source's lessons as its only rules
//! block — the reflector's lesson for this intervention, and the lessons of
//! the session's text appraisal. Both lesson arms are rendered in the frame
//! a learned rule rides in ([`lesson_block`]), under the reflection's
//! domain, so the two differ in the lessons and nothing else.
//!
//! **Clean only, on both sides.** The reflector's lesson passes the gate
//! `mecha learn` applies ([`Reflexion::learnable`]) and is the reflector's
//! own words — an owner-edited lesson is the owner's, and would credit the
//! reflector with them. The appraisal's lessons come through the clean door
//! (R19, [`crate::appraisal_store::Clean`]). An intervention clean for one
//! source and not the other is excluded and counted ([`Exclusion`]); the
//! store's own rule then refuses anything else it would not keep.
//!
//! **The report** ([`report`]) is per intervention region — the scope key of
//! the reflection's recorded situation (§17.4, [`Situation::key`], the key a
//! validation row's region is folded on). For each region: each source's
//! validation rate — the share of decided interventions at which the arm
//! carrying that source's lessons did what the owner's recorded intervention
//! asked for — with its counts beneath it, the rules-free arm's beside them,
//! and the inconclusive, unmeasured, unavailable and excluded counts. A rate
//! over nothing decided is `None`. The verdicts themselves are stored as 1g
//! comparisons ([`Kind::LessonSource`]), so the report is re-read from the
//! stores, never from a pass's memory.

use crate::appraisal_store::{Clean, CleanRead};
use crate::comparison::{Arm, Comparison, Kind, Outcome, Role, Verdict};
use crate::learning::{rules_hash, wrap_rules_block, Reflexion, Trigger, RUN_DOMAINS};
use crate::situation::Situation;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Most interventions one pass drives, by default — each holds one
/// background seat for three arms.
pub const DEFAULT_INTERVENTIONS: usize = 8;

/// Why an intervention the reflector reflected on is not compared. Each is
/// counted apart, because they call for different fixes — and "clean for
/// one source only" is the finding the design asks to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Exclusion {
    /// A followup (judge-graded — R27 lets a judge decide nothing), an
    /// outbox edit or a rejection (no replayable point), a mismatch (graded
    /// against an owner's artifact fixture, not by the intervention).
    NotTraceGraded,
    /// A reflection outside [`RUN_DOMAINS`]: its lesson may not ride in
    /// front of a tool-having probe (`RuleSurface::load`'s argument).
    NotARunDomain,
    /// The owner dropped the reflection: it is no longer a lesson of anyone's.
    DroppedByOwner,
    /// The owner rewrote the lesson: the words are the owner's, and scoring
    /// them as the reflector's would credit it with them.
    EditedByOwner,
    /// No text appraisal of the session is on record.
    NoAppraisal,
    /// The reflection passes `learn`'s gate; the session's appraisal is not
    /// served by the clean door.
    CleanForReflectorOnly,
    /// The appraisal is clean; the reflection does not pass `learn`'s gate.
    CleanForAppraisalOnly,
    CleanForNeither,
    /// The reflection carries no lesson text.
    ReflectorWithoutLesson,
    /// The clean appraisal carries no lessons.
    AppraisalWithoutLessons,
}

/// One intervention both sources may speak to: the reflection, and the
/// session's clean appraisal.
#[derive(Debug, Clone, Copy)]
pub struct Pair<'a> {
    pub reflection: &'a Reflexion,
    pub appraisal: &'a Clean,
}

/// A source's lessons as a probe arm carries them: the rules block a learned
/// rule set would render, with only these lines, under `domain`. Each lesson
/// is one line — whitespace runs, newlines included, collapsed — so no
/// lesson can open a heading of its own. `None` when no line is left.
pub fn lesson_block(domain: &str, lessons: &[&str]) -> Option<String> {
    let lines: Vec<String> = lessons
        .iter()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .map(|l| format!("- {l}"))
        .collect();
    if lines.is_empty() {
        return None;
    }
    wrap_rules_block(vec![format!("### {domain}\n{}", lines.join("\n"))])
}

impl<'a> Pair<'a> {
    /// The reflector's lesson for this intervention, as its arm carries it.
    pub fn reflector_block(&self) -> Option<String> {
        lesson_block(
            &self.reflection.domain,
            &[self.reflection.reflexion_text.as_str()],
        )
    }

    /// The session appraisal's lessons, as their arm carries them.
    pub fn appraisal_block(&self) -> Option<String> {
        let lessons: Vec<&str> = self.appraisal.lessons.iter().map(String::as_str).collect();
        lesson_block(&self.reflection.domain, &lessons)
    }

    /// The three arms, in the order they are driven and stored: rules-free,
    /// the reflector's, the appraisal's. `None` when either source has no
    /// line to carry.
    pub fn arms(&self) -> Option<[LessonArm; 3]> {
        let reflector = self.reflector_block()?;
        let appraisal = self.appraisal_block()?;
        let arm = |role, block: Option<String>| LessonArm {
            role,
            policy: match &block {
                Some(b) => Some(rules_hash(b)),
                None => Arm::no_block(),
            },
            block,
        };
        Some([
            arm(Role::RulesFree, None),
            arm(Role::ReflectorLesson, Some(reflector)),
            arm(Role::AppraisalLesson, Some(appraisal)),
        ])
    }
}

/// One arm of a lesson comparison: its role, its policy hash
/// (`RunConfig::rules_hash`'s convention — no block is the empty string's
/// hash) and the rules block it carries, `None` for rules-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LessonArm {
    pub role: Role,
    pub policy: Option<String>,
    pub block: Option<String>,
}

/// What the two sources hold, read once: the clean door's appraisals by
/// session, and which sessions have *any* appraisal on record — ids only,
/// so a withheld appraisal is counted without its text ever being read.
pub struct Sources<'a> {
    clean: BTreeMap<&'a str, &'a Clean>,
    on_record: BTreeSet<String>,
}

impl<'a> Sources<'a> {
    pub fn new(clean: &'a CleanRead, on_record: BTreeSet<String>) -> Sources<'a> {
        Sources {
            clean: clean
                .appraisals
                .iter()
                .map(|c| (c.session_id.as_str(), c))
                .collect(),
            on_record,
        }
    }

    /// The pair `r` makes, or why it makes none.
    pub fn pair<'r>(&self, r: &'r Reflexion) -> Result<Pair<'r>, Exclusion>
    where
        'a: 'r,
    {
        let traced = r.trigger == Trigger::Steer.as_str() || r.trigger == Trigger::Denial.as_str();
        if !traced {
            return Err(Exclusion::NotTraceGraded);
        }
        if !RUN_DOMAINS.contains(&r.domain.as_str()) {
            return Err(Exclusion::NotARunDomain);
        }
        if r.dropped_at.is_some() {
            return Err(Exclusion::DroppedByOwner);
        }
        if r.edited_at.is_some() {
            return Err(Exclusion::EditedByOwner);
        }
        let appraisal = self.clean.get(r.session_id.as_str()).copied();
        if appraisal.is_none() && !self.on_record.contains(&r.session_id) {
            return Err(Exclusion::NoAppraisal);
        }
        let appraisal = match (r.learnable(), appraisal) {
            (true, Some(a)) => a,
            (true, None) => return Err(Exclusion::CleanForReflectorOnly),
            (false, Some(_)) => return Err(Exclusion::CleanForAppraisalOnly),
            (false, None) => return Err(Exclusion::CleanForNeither),
        };
        let pair = Pair {
            reflection: r,
            appraisal,
        };
        if pair.reflector_block().is_none() {
            return Err(Exclusion::ReflectorWithoutLesson);
        }
        if pair.appraisal_block().is_none() {
            return Err(Exclusion::AppraisalWithoutLessons);
        }
        Ok(pair)
    }
}

/// The region an intervention is reported under: its recorded situation's
/// scope key, `None` when the reflection predates situations — unknown,
/// never standing (an empty key is standing, and matches every run).
pub fn region_of(situation: Option<&Situation>) -> Option<String> {
    situation.map(Situation::key)
}

/// The newest stored comparison that measured `pair`'s lessons as they
/// stand, under `model`: this reflection, this kind, and the same three
/// (role, policy) arms. A comparison of a lesson since rewritten measured
/// other words, so it is not this pair's.
pub fn on_record<'c>(
    rows: &'c [Comparison],
    pair: &Pair<'_>,
    model: &str,
) -> Option<&'c Comparison> {
    let wanted: BTreeSet<(String, Option<String>)> = pair
        .arms()?
        .iter()
        .map(|a| (crate::appraisal::enum_name(&a.role), a.policy.clone()))
        .collect();
    rows.iter().rev().find(|c| {
        c.kind == Kind::LessonSource
            && c.model == model
            && c.pointers.reflection_id.as_deref() == Some(pair.reflection.id.as_str())
            && c.arms.len() == wanted.len()
            && c.arms
                .iter()
                .map(|a| (crate::appraisal::enum_name(&a.role), a.policy.clone()))
                .collect::<BTreeSet<_>>()
                == wanted
    })
}

/// One source's arm, counted over a region's comparisons.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SourceCounts {
    /// Of the decided comparisons: the arm did what the owner's recorded
    /// intervention asked for, or did not.
    pub pass: usize,
    pub fail: usize,
    /// Of the decided comparisons, against the rules-free arm at the same
    /// intervention: the lessons turned a failure into a pass, or the
    /// reverse. Always zero for the rules-free arm itself.
    pub improved: usize,
    pub regressed: usize,
    /// Comparisons in which this arm posed no question (or one this build
    /// cannot read) — the reason the comparison is undecided.
    pub inconclusive: usize,
}

impl SourceCounts {
    /// The validation rate: passes over decided interventions. `None` over
    /// nothing decided — a dash, never zero.
    pub fn rate(&self) -> Option<f64> {
        let decided = self.pass + self.fail;
        (decided > 0).then(|| self.pass as f64 / decided as f64)
    }
}

/// One region's report.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RegionReport {
    /// [`Situation::key`] of the region; `None` is unknown (a reflection
    /// from before situations), and the empty string is standing.
    pub region: Option<String>,
    /// The region in words, for a readout.
    pub describe: String,
    /// Interventions both sources may speak to (clean on both sides, a
    /// lesson on each).
    pub eligible: usize,
    /// Of those, the ones whose lessons as they stand have a comparison on
    /// record under this model.
    pub compared: usize,
    /// Of those, the ones every arm graded: the paired set every rate below
    /// is over.
    pub decided: usize,
    /// Compared, and some arm posed no question: evidence for no arm.
    pub inconclusive: usize,
    /// Eligible with no comparison on record: never drawn, over a budget,
    /// or unavailable on every pass that drew it.
    pub unmeasured: usize,
    /// Drawn this pass and refused before any arm was driven (the recording
    /// cannot be located or replayed, its tool surface is gone, the store
    /// would not keep the verdict) or lost to an arm that could not be
    /// driven. `None` outside a pass: nothing keeps it, and unknown is not
    /// zero.
    pub unavailable: Option<usize>,
    pub reflector: SourceCounts,
    pub appraisal: SourceCounts,
    /// The recorded prompt with no rules: what both sources are measured
    /// against. Its `improved` and `regressed` are zero by construction.
    pub rules_free: SourceCounts,
    /// Interventions in this region not compared, by why.
    pub excluded: BTreeMap<Exclusion, usize>,
}

/// The whole report: per region, for one model.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    /// The model whose comparisons are counted; `None` when no lesson
    /// comparison is on record at all.
    pub model: Option<String>,
    /// Lesson comparisons on record under other models — not counted,
    /// since comparisons are comparable only within one.
    pub other_models: usize,
    pub regions: Vec<RegionReport>,
    /// Torn lines skipped in the three stores the report reads —
    /// reflections, appraisals, comparisons. Set by the caller that read
    /// them; nonzero makes every count a floor, and can move an exclusion
    /// (a torn appraisal line reads as `NoAppraisal`), so a reader must not
    /// call the report complete.
    pub skipped_lines: usize,
}

impl Report {
    /// The totals across regions, in one [`RegionReport`] with no region.
    pub fn total(&self) -> RegionReport {
        let mut t = RegionReport {
            describe: "all regions".into(),
            ..RegionReport::default()
        };
        for r in &self.regions {
            t.eligible += r.eligible;
            t.compared += r.compared;
            t.decided += r.decided;
            t.inconclusive += r.inconclusive;
            t.unmeasured += r.unmeasured;
            t.unavailable = match (t.unavailable, r.unavailable) {
                (Some(a), Some(b)) => Some(a + b),
                (a, b) => a.or(b),
            };
            for (mine, theirs) in [
                (&mut t.reflector, &r.reflector),
                (&mut t.appraisal, &r.appraisal),
                (&mut t.rules_free, &r.rules_free),
            ] {
                mine.pass += theirs.pass;
                mine.fail += theirs.fail;
                mine.improved += theirs.improved;
                mine.regressed += theirs.regressed;
                mine.inconclusive += theirs.inconclusive;
            }
            for (why, n) in &r.excluded {
                *t.excluded.entry(*why).or_default() += n;
            }
        }
        t
    }
}

/// An arm's outcome by role, or `Unknown` when the comparison has none.
fn outcome_of(c: &Comparison, role: Role) -> Outcome {
    c.arms
        .iter()
        .find(|a| a.role == role)
        .map(|a| a.outcome)
        .unwrap_or(Outcome::Unknown)
}

fn fold(region: &mut RegionReport, c: &Comparison) {
    region.compared += 1;
    let base = outcome_of(c, Role::RulesFree);
    let arms = [
        (Role::RulesFree, base),
        (Role::ReflectorLesson, outcome_of(c, Role::ReflectorLesson)),
        (Role::AppraisalLesson, outcome_of(c, Role::AppraisalLesson)),
    ];
    let decided = matches!(c.verdict, Verdict::Separated | Verdict::Tied)
        && arms
            .iter()
            .all(|(_, o)| matches!(o, Outcome::Pass | Outcome::Fail));
    for (role, outcome) in arms {
        let counts = match role {
            Role::RulesFree => &mut region.rules_free,
            Role::ReflectorLesson => &mut region.reflector,
            _ => &mut region.appraisal,
        };
        if !matches!(outcome, Outcome::Pass | Outcome::Fail) {
            counts.inconclusive += 1;
        }
        if !decided {
            continue;
        }
        match outcome {
            Outcome::Pass => counts.pass += 1,
            _ => counts.fail += 1,
        }
        if role != Role::RulesFree {
            match (base, outcome) {
                (Outcome::Fail, Outcome::Pass) => counts.improved += 1,
                (Outcome::Pass, Outcome::Fail) => counts.regressed += 1,
                _ => {}
            }
        }
    }
    if decided {
        region.decided += 1;
    } else {
        region.inconclusive += 1;
    }
}

/// The report, from the stores: every reflection classified, each eligible
/// one matched to the newest comparison of its lessons as they stand under
/// `model` — or, when `model` is `None`, under the model of the newest
/// lesson comparison on record. `unavailable`, by region key, is a pass's
/// own count; `None` outside a pass.
pub fn report(
    reflections: &[Reflexion],
    sources: &Sources<'_>,
    comparisons: &[Comparison],
    model: Option<&str>,
    unavailable: Option<&BTreeMap<Option<String>, usize>>,
) -> Report {
    let model: Option<String> = model.map(str::to_owned).or_else(|| {
        comparisons
            .iter()
            .rev()
            .find(|c| c.kind == Kind::LessonSource)
            .map(|c| c.model.clone())
    });
    let other_models = comparisons
        .iter()
        .filter(|c| c.kind == Kind::LessonSource && Some(&c.model) != model.as_ref())
        .count();
    let mut regions: BTreeMap<Option<String>, RegionReport> = BTreeMap::new();
    let mut entry = |situation: Option<&Situation>| -> Option<String> {
        let key = region_of(situation);
        regions.entry(key.clone()).or_insert_with(|| RegionReport {
            region: key.clone(),
            describe: match situation {
                Some(s) => s.scope().describe(),
                None => "unknown (recorded before situations)".into(),
            },
            unavailable: unavailable.map(|_| 0),
            ..RegionReport::default()
        });
        key
    };
    let mut keyed: Vec<(Option<String>, Result<Pair<'_>, Exclusion>)> = Vec::new();
    for r in reflections {
        let key = entry(r.situation.as_ref());
        keyed.push((key, sources.pair(r)));
    }
    if let Some(map) = unavailable {
        for key in map.keys() {
            regions.entry(key.clone()).or_insert_with(|| RegionReport {
                region: key.clone(),
                describe: key.clone().unwrap_or_else(|| "unknown".into()),
                unavailable: Some(0),
                ..RegionReport::default()
            });
        }
    }
    for (key, pair) in keyed {
        let region = regions.get_mut(&key).expect("entered above");
        match pair {
            Err(why) => *region.excluded.entry(why).or_default() += 1,
            Ok(pair) => {
                region.eligible += 1;
                match model
                    .as_deref()
                    .and_then(|m| on_record(comparisons, &pair, m))
                {
                    Some(c) => fold(region, c),
                    None => region.unmeasured += 1,
                }
            }
        }
    }
    if let Some(map) = unavailable {
        for (key, n) in map {
            if let Some(region) = regions.get_mut(key) {
                region.unavailable = Some(region.unavailable.unwrap_or(0) + n);
            }
        }
    }
    Report {
        model,
        other_models,
        regions: regions.into_values().collect(),
        skipped_lines: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comparison::{Pointers, Validator};
    use crate::learning::Origin;

    fn reflection(id: &str, session: &str, trigger: &str, tools: &[&str]) -> Reflexion {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "domain": "behavior",
            "session_id": session,
            "trigger": trigger,
            "context": "fs_write q3.md",
            "intervention": "not that file",
            "reflexion_text": "Ask before overwriting a quarter's file.",
            "error_type": null,
            "confidence": null,
            "created_at": "2026-09-25T00:00:00Z",
            "origin": "clean",
            "situation": Situation::recorded(
                &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                trigger,
                None,
                None,
            ),
        }))
        .unwrap()
    }

    fn clean_read(rows: Vec<crate::appraisal_store::TextAppraisal>) -> CleanRead {
        crate::appraisal_store::clean_read_of(rows)
    }

    fn appraisal(session: &str, clean: bool) -> crate::appraisal_store::TextAppraisal {
        crate::appraisal_store::test_row(
            session,
            &Situation::default(),
            clean,
            "2026-09-25T00:00:00Z",
        )
    }

    fn comparison(r: &Reflexion, pair: &Pair<'_>, outcomes: [Outcome; 3]) -> Comparison {
        let arms = pair
            .arms()
            .unwrap()
            .into_iter()
            .zip(outcomes)
            .map(|(a, o)| Arm::new(a.role, a.policy, o))
            .collect();
        Comparison::new(
            Kind::LessonSource,
            r.situation.clone(),
            None,
            None,
            arms,
            Validator::StructuralDenial,
            Pointers {
                session_id: r.session_id.clone(),
                reflection_id: Some(r.id.clone()),
                ..Pointers::default()
            },
            "local-model",
        )
    }

    /// Every exclusion is its own count, and the clean-for-one-side cases
    /// are told apart from each other and from "no appraisal at all".
    #[test]
    fn each_way_of_not_pairing_is_named() {
        let read = clean_read(vec![
            appraisal("s-clean", true),
            appraisal("s-tainted", false),
            {
                let mut a = appraisal("s-bare", true);
                a.lessons.clear();
                a
            },
        ]);
        let on_record: BTreeSet<String> = ["s-clean", "s-tainted", "s-bare"]
            .into_iter()
            .map(String::from)
            .collect();
        let sources = Sources::new(&read, on_record);
        let ok = reflection("r1", "s-clean", "denial", &["fs_write"]);
        assert!(sources.pair(&ok).is_ok());
        let cases: Vec<(Reflexion, Exclusion)> = vec![
            (
                reflection("r2", "s-clean", "followup", &[]),
                Exclusion::NotTraceGraded,
            ),
            (
                reflection("r3", "s-clean", "edit", &[]),
                Exclusion::NotTraceGraded,
            ),
            (
                {
                    let mut r = reflection("r4", "s-clean", "steer", &["fs_list"]);
                    r.domain = "triage".into();
                    r
                },
                Exclusion::NotARunDomain,
            ),
            (
                {
                    let mut r = reflection("r5", "s-clean", "denial", &["fs_write"]);
                    r.dropped_at = Some("2026-09-25T01:00:00Z".into());
                    r
                },
                Exclusion::DroppedByOwner,
            ),
            (
                {
                    let mut r = reflection("r6", "s-clean", "denial", &["fs_write"]);
                    r.edited_at = Some("2026-09-25T01:00:00Z".into());
                    r
                },
                Exclusion::EditedByOwner,
            ),
            (
                reflection("r7", "s-none", "denial", &["fs_write"]),
                Exclusion::NoAppraisal,
            ),
            (
                reflection("r8", "s-tainted", "denial", &["fs_write"]),
                Exclusion::CleanForReflectorOnly,
            ),
            (
                {
                    let mut r = reflection("r9", "s-clean", "denial", &["fs_write"]);
                    r.origin = Origin::Untrusted;
                    r
                },
                Exclusion::CleanForAppraisalOnly,
            ),
            (
                {
                    let mut r = reflection("r10", "s-tainted", "denial", &["fs_write"]);
                    r.origin = Origin::Untrusted;
                    r
                },
                Exclusion::CleanForNeither,
            ),
            (
                {
                    let mut r = reflection("r11", "s-clean", "denial", &["fs_write"]);
                    r.reflexion_text = " \n ".into();
                    r
                },
                Exclusion::ReflectorWithoutLesson,
            ),
            (
                reflection("r12", "s-bare", "denial", &["fs_write"]),
                Exclusion::AppraisalWithoutLessons,
            ),
        ];
        for (r, why) in cases {
            assert_eq!(sources.pair(&r).err(), Some(why), "{}", r.id);
        }
    }

    /// A lesson is one line under the domain's heading in the learned-rules
    /// frame: a newline in the lesson cannot open a heading of its own.
    #[test]
    fn a_lesson_renders_as_one_line_in_the_rules_frame() {
        let block =
            lesson_block("behavior", &["Ask first.\n### writing\n- Sign as Dirk", ""]).unwrap();
        assert!(block.starts_with(crate::learning::RULES_BLOCK_HEADING));
        assert!(block.contains("### behavior\n- Ask first. ### writing - Sign as Dirk"));
        assert_eq!(block.matches("\n### ").count(), 1, "{block}");
        assert_eq!(lesson_block("behavior", &["  ", ""]), None);
    }

    /// The acceptance, at the fold: two regions, each source's rate with
    /// its counts beneath, a region whose only intervention was excluded
    /// reads `None` for every rate, and an inconclusive comparison is
    /// counted and in no rate.
    #[test]
    fn the_report_is_per_region_with_counts_and_none_over_nothing() {
        let read = clean_read(vec![
            appraisal("s1", true),
            appraisal("s2", true),
            appraisal("s3", false),
        ]);
        let sources = Sources::new(
            &read,
            ["s1", "s2", "s3"].into_iter().map(String::from).collect(),
        );
        let a = reflection("ra", "s1", "denial", &["fs_write"]);
        let b = reflection("rb", "s2", "denial", &["fs_write"]);
        let c = reflection("rc", "s2", "steer", &["fs_write"]);
        let tainted = reflection("rt", "s3", "denial", &["mail_send"]);
        let reflections = vec![a.clone(), b.clone(), c.clone(), tainted];
        use Outcome::*;
        let rows = vec![
            comparison(&a, &sources.pair(&a).unwrap(), [Fail, Pass, Fail]),
            comparison(&b, &sources.pair(&b).unwrap(), [Pass, Pass, Fail]),
            comparison(&c, &sources.pair(&c).unwrap(), [Fail, Inconclusive, Pass]),
        ];
        let report = report(&reflections, &sources, &rows, None, None);
        assert_eq!(report.model.as_deref(), Some("local-model"));
        assert_eq!(report.regions.len(), 2, "{report:#?}");
        let write = report
            .regions
            .iter()
            .find(|r| r.region.as_deref() == Some("fs_write"))
            .unwrap();
        assert_eq!(
            (
                write.eligible,
                write.compared,
                write.decided,
                write.inconclusive
            ),
            (3, 3, 2, 1)
        );
        assert_eq!(write.reflector.rate(), Some(1.0));
        assert_eq!(write.appraisal.rate(), Some(0.0));
        assert_eq!(write.rules_free.rate(), Some(0.5));
        assert_eq!(
            write.reflector,
            SourceCounts {
                pass: 2,
                fail: 0,
                improved: 1,
                regressed: 0,
                inconclusive: 1
            }
        );
        assert_eq!(
            write.appraisal,
            SourceCounts {
                pass: 0,
                fail: 2,
                improved: 0,
                regressed: 1,
                inconclusive: 0
            }
        );
        assert_eq!(write.unavailable, None, "outside a pass it is unknown");
        let mail = report
            .regions
            .iter()
            .find(|r| r.region.as_deref() == Some("mail_send"))
            .unwrap();
        assert_eq!(mail.eligible, 0);
        assert_eq!(
            (
                mail.reflector.rate(),
                mail.appraisal.rate(),
                mail.rules_free.rate()
            ),
            (None, None, None),
            "a rate over nothing decided is None, not zero"
        );
        assert_eq!(
            mail.excluded.get(&Exclusion::CleanForReflectorOnly),
            Some(&1)
        );
        let total = report.total();
        assert_eq!((total.eligible, total.decided), (3, 2));
        assert_eq!(
            total.excluded.get(&Exclusion::CleanForReflectorOnly),
            Some(&1)
        );
    }

    /// A comparison counts for an intervention only while it measured the
    /// lessons as they stand, under the model asked about: a rewritten
    /// appraisal lesson or another model's row leaves it unmeasured.
    #[test]
    fn a_comparison_of_other_words_or_another_model_is_not_this_pairs() {
        let read = clean_read(vec![appraisal("s1", true)]);
        let sources = Sources::new(&read, ["s1".to_string()].into_iter().collect());
        let r = reflection("ra", "s1", "denial", &["fs_write"]);
        let pair = sources.pair(&r).unwrap();
        use Outcome::*;
        let row = comparison(&r, &pair, [Fail, Pass, Pass]);
        assert!(on_record(std::slice::from_ref(&row), &pair, "local-model").is_some());
        assert!(on_record(std::slice::from_ref(&row), &pair, "other-model").is_none());

        let mut rewritten = r.clone();
        rewritten.reflexion_text = "Confirm the quarter before writing.".into();
        let pair2 = sources.pair(&rewritten).unwrap();
        assert!(on_record(std::slice::from_ref(&row), &pair2, "local-model").is_none());

        let rep = report(
            std::slice::from_ref(&r),
            &sources,
            &[row],
            Some("other-model"),
            Some(&BTreeMap::from([(Some("fs_write".to_string()), 2)])),
        );
        assert_eq!(rep.other_models, 1);
        let region = &rep.regions[0];
        assert_eq!((region.compared, region.unmeasured), (0, 1));
        assert_eq!(region.unavailable, Some(2));
    }
}
