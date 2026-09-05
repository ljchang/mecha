//! The charter: what mecha is for, in the owner's own words.
//!
//! `docs/GOAL-SYSTEM-DESIGN.md` §11 is the design. The short form: a small,
//! ordered list of standing priorities, written once by a person and read by
//! every run from then on. It exists so that a goal error can eventually be
//! signed *against* something — "did this help what mecha is for" — rather
//! than every evaluative signal in the system being a cost or a correction,
//! which is the gap the whole goal-system arc is closing.
//!
//! ## The safety argument is the skills argument, verbatim
//!
//! No `mecha charter learn`, no registry, nothing derived from a session, and
//! **no way for a model to author or edit one** — see [`crate::skill`]'s
//! module doc for the incident this is copied from (Snyk found 36.8% of
//! published Agent Skills carrying a security flaw; Datadog's sharper finding
//! is that a cloned repository can bring one into a trusted session even
//! without an install step). A model that could edit its own charter could
//! edit its way around every other guardrail.
//!
//! **The invariant is about the author, not about the verb**, and this file
//! used to state it as the latter — "there is deliberately no write path here
//! at all". That was wrong by the time it was written: the TUI's `/charter`
//! already handed the file to `$EDITOR`, and the web settings page already
//! POSTed a validated save. What every one of those surfaces has in common is
//! the thing that actually matters, and it is worth stating positively so the
//! next surface copies the right rule:
//!
//! > **The owner may edit the charter from anywhere. Every `[[line]]` is
//! > typed by a person, and no model — privileged, quarantined or otherwise —
//! > ever composes, suggests or edits one.**
//!
//! So a surface may create the comments-only [`TEMPLATE`] and hand over an
//! editor; it may validate and refuse; it may not put words in the file. This
//! module itself only ever *reads*: the write is the owner's editor or a
//! validated save at a surface, never a derivation in here, which is what
//! keeps "a model authored this" impossible rather than merely discouraged.
//!
//! **Global only, and there is no config field to point it elsewhere.** A
//! `mecha.toml` arrives with a cloned repository, and a repo that could hand
//! your agent standing priorities is the `[[trigger]]` rule in a worse
//! costume. Triggers and skills both keep this guarantee by never having a
//! configurable path in `Config`/`ConfigLayer` at all rather than by relying
//! on callers to choose the global loader, and this module follows the same
//! shape: [`Charter::default_path`] is the only path there is.
//!
//! **Loading it arms no taint.** It is the user's own words, exactly like the
//! system prompt — the same argument `crate::skill` makes, and for the same
//! reason this module has no dependency on `crate::agent::Taint` at all: the
//! absence is the enforcement, not a rule someone has to remember to apply.
//!
//! ## Ordered, not weighted
//!
//! §11 is explicit that priority is the file's line order and there is no
//! priority field: value conflict — "protect the owner" against "don't let a
//! colleague down" — is the measured cause of goal drift, and a weighted sum
//! can always be outvoted by enough small goods (*"this is urgent for many
//! people"*). A lexicographic order cannot be outvoted that way, so this
//! module preserves the TOML array's order exactly rather than sorting it —
//! unlike [`crate::skill::SkillStore`], which sorts because its block is a
//! menu the model chooses from and filesystem order is not an order. A
//! charter's order *is* the content.
//!
//! ## Rendered directly, never lazily loaded
//!
//! Unlike a skill, there is no progressive disclosure and no tool call: §11
//! says this rides in the cached prefix "like `RULES_CHAR_BUDGET`" — i.e. it
//! is rendered straight into the system prompt every run, the same way the
//! learned-rules block is, because a handful of standing priorities is cheap
//! enough to always carry and too important to make conditional on the model
//! deciding to ask for it.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The charter's whole rendered form is meant to fit this many characters —
/// checked by [`Charter::over_budget`], never enforced by [`Charter::load`].
///
/// Moves with [`crate::learning::RULES_CHAR_BUDGET`] in spirit — this rides
/// in every run's cached prefix exactly like the learned-rules block — but is
/// smaller: a charter is a handful of standing priorities, not a per-domain
/// accumulation, and a lexicographic order only stays legible while it stays
/// short enough for a person to hold in mind at once. Argued, not measured:
/// there is no corpus yet of how many lines a charter needs before it stops
/// being read carefully.
pub const CHARTER_CHAR_BUDGET: usize = 2000;

/// One standing priority.
///
/// Built from a [`RawLine`] by [`Charter::validate`], never deserialised
/// directly: the sensor's setpoint is typed by its kind, and that check has
/// to run against the kind beside it, which `serde` cannot express per field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharterLine {
    /// What a [`crate::goal::GoalRef::Charter`] names. Unique within a
    /// charter — see [`Charter::load`].
    pub id: String,
    pub text: String,
    /// An observable the harness reads from its own stores, with a setpoint
    /// the owner wrote — `docs/GOAL-SYSTEM-DESIGN.md` §11.1. Most lines have
    /// none, and a line without one is not the lesser kind (containment 4):
    /// it still counts through the task tier and the closure appraisal.
    pub sensor: Option<Sensor>,
}

/// The `[[line]]` table as the file spells it.
///
/// **Denies unknown fields**, unlike [`crate::skill::Skill`]'s frontmatter —
/// that leniency is for portability across harnesses that might author a
/// `SKILL.md`, and nothing else authors a `charter.toml`. A stray `priority`
/// or `rank` key is exactly the field §11 says there deliberately is none of;
/// silently dropping it would let an owner write one, believe it did
/// something, and never find out it didn't.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawLine {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub sensor: Option<RawSensor>,
}

/// The `[line.sensor]` table as the file spells it: a kind word from the
/// closed set, and a setpoint the kind decides how to read.
///
/// `setpoint` accepts a TOML string or a bare number, because `setpoint = 3`
/// is how an owner naturally writes a count; both are kept as the owner's
/// own spelling and typed by [`SensorKind::parse_setpoint`] at validation.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawSensor {
    pub kind: SensorKind,
    pub setpoint: RawSetpoint,
}

/// A setpoint before its kind has typed it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RawSetpoint {
    Text(String),
    Integer(i64),
    Float(f64),
}

impl RawSetpoint {
    /// The owner's spelling, as one string. A number is rendered the way
    /// TOML would print it, so the web editor's round trip (`setpoint = 3`
    /// in, `setpoint = "3"` out) changes the quoting and nothing else.
    fn text(&self) -> String {
        match self {
            RawSetpoint::Text(s) => s.trim().to_string(),
            RawSetpoint::Integer(n) => n.to_string(),
            RawSetpoint::Float(f) => f.to_string(),
        }
    }
}

/// What a sensored line watches — **a closed set the owner picks from**,
/// never an expression, a command or a path, so the file stays a wire format
/// `Charter::load` can refuse exactly as it refuses an unknown key. Every
/// kind here is an observable a store already holds *with an id per item*
/// that a run's own trace can touch, because attribution
/// (`appraisal::of_session`) joins on that id, never on a before/after delta
/// of the store (§11.1, containment 6) — and **every kind here does
/// something today**: each is a key in `sensor_kinds_for`'s table. §11.1
/// also names `board_overdue` and `cost`, which are store- and run-level
/// numbers with no item a trace touches; they can only ever be *reading*
/// sensors, and the readings are the section's unbuilt phase. They are
/// deliberately not variants yet: a kind that parses, validates its setpoint
/// and then does nothing is the failure `RawLine`'s `deny_unknown_fields`
/// exists to refuse, one field down (found on review). They join when a
/// reader does.
///
/// A kind this binary does not know is a load error, which is the
/// fail-closed direction the charter already has. On an older binary that
/// is **not** a startup refusal — `setup.rs` catches every `Charter::load`
/// error, prints one stderr line (covered by the TUI's alternate screen) and
/// runs *un-chartered*, so a sensored line authored here silently costs a
/// machine on the previous release its whole charter until `mecha doctor`
/// reports it, which it does at the severity it deserves. The fix is the
/// `update` skill, not a lenient parser; §11.1's containment 7 says
/// "refusal" and is corrected here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    /// How many outbox drafts are waiting on the owner. A count.
    OutboxWaiting,
    /// How long a staged draft has sat unreviewed. A duration.
    OutboxAge,
    /// How long a parked question waits for the owner's answer. A duration.
    QuestionLatency,
    /// How long a front-door request stays open before it is closed or
    /// answered. A duration.
    RequestClosure,
    /// The share of runs in which the owner had to step in. A rate.
    InterventionRate,
}

/// The unit a kind's setpoint is read in — fixed by the kind, never chosen
/// in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Duration,
    Count,
    Rate,
}

impl Unit {
    /// The wire word, for a surface that lists what an owner may pick from.
    pub fn wire(self) -> &'static str {
        match self {
            Unit::Duration => "duration",
            Unit::Count => "count",
            Unit::Rate => "rate",
        }
    }

    /// How a setpoint in this unit is spelled — the one sentence the parse
    /// error and the web form's hint both use, so the form never proposes
    /// a number and the refusal never disagrees with the hint.
    pub fn hint(self) -> &'static str {
        match self {
            Unit::Duration => "a duration like `24h` or `7d`",
            Unit::Count => "a whole number",
            Unit::Rate => "a rate like `0.2` or `20%`",
        }
    }
}

/// A setpoint typed by its kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Setpoint {
    Duration(std::time::Duration),
    Count(u64),
    /// A share in `0.0..=1.0`.
    Rate(f64),
}

impl Setpoint {
    fn is_zero(self) -> bool {
        match self {
            Setpoint::Duration(d) => d.is_zero(),
            Setpoint::Count(n) => n == 0,
            Setpoint::Rate(r) => r <= 0.0,
        }
    }
}

impl SensorKind {
    /// Every kind, for a surface that lists what an owner may pick from.
    pub const ALL: [SensorKind; 5] = [
        SensorKind::OutboxWaiting,
        SensorKind::OutboxAge,
        SensorKind::QuestionLatency,
        SensorKind::RequestClosure,
        SensorKind::InterventionRate,
    ];

    /// The wire word — `serde`'s own `snake_case` spelling, for a message
    /// or a JSON field that wants a bare `&str`.
    pub fn wire(self) -> &'static str {
        match self {
            SensorKind::OutboxWaiting => "outbox_waiting",
            SensorKind::OutboxAge => "outbox_age",
            SensorKind::QuestionLatency => "question_latency",
            SensorKind::RequestClosure => "request_closure",
            SensorKind::InterventionRate => "intervention_rate",
        }
    }

    pub fn unit(self) -> Unit {
        match self {
            SensorKind::OutboxAge | SensorKind::QuestionLatency | SensorKind::RequestClosure => {
                Unit::Duration
            }
            SensorKind::OutboxWaiting => Unit::Count,
            SensorKind::InterventionRate => Unit::Rate,
        }
    }

    /// What the kind watches, in one line, for a surface that offers the
    /// closed set to the owner — the variant docs above, as prose.
    pub fn describe(self) -> &'static str {
        match self {
            SensorKind::OutboxWaiting => "how many outbox drafts wait on you",
            SensorKind::OutboxAge => "how long a staged draft has sat unreviewed",
            SensorKind::QuestionLatency => "how long a parked question waits for your answer",
            SensorKind::RequestClosure => "how long a front-door request stays open",
            SensorKind::InterventionRate => "the share of recent runs you stepped into",
        }
    }

    /// Read a setpoint in this kind's unit, or say what the unit is.
    ///
    /// Durations are `<n><unit>` tokens — `24h`, `90m`, `7d`, `1h30m` — over
    /// `s`, `m`, `h`, `d`, `w`; counts are whole numbers; a rate is `0.2` or
    /// `20%`. Strict on purpose:
    /// a setpoint of one hour where the owner meant one day saturates a
    /// reading (containment 5), and the place to catch a unit the owner did
    /// not mean is the parse, where the error names the line.
    pub fn parse_setpoint(self, text: &str) -> Result<Setpoint> {
        let text = text.trim();
        if text.is_empty() {
            bail!("setpoint is empty");
        }
        let setpoint = match self.unit() {
            Unit::Duration => parse_duration(text).map(Setpoint::Duration)?,
            Unit::Count => text
                .parse::<u64>()
                .map(Setpoint::Count)
                .map_err(|_| anyhow::anyhow!("`{text}` is not a whole number"))?,
            Unit::Rate => {
                let (body, scale) = match text.strip_suffix('%') {
                    Some(pct) => (pct.trim(), 0.01),
                    None => (text, 1.0),
                };
                let n: f64 = body
                    .parse()
                    .map_err(|_| anyhow::anyhow!("`{text}` is not a rate like `0.2` or `20%`"))?;
                let rate = n * scale;
                if !(0.0..=1.0).contains(&rate) {
                    bail!("`{text}` is not a share between 0 and 1");
                }
                Setpoint::Rate(rate)
            }
        };
        // A setpoint of zero is a line nothing could ever be within: one
        // draft, one second, one intervention in a hundred runs all read as
        // past it, and the reading is a constant on every run — the
        // saturated number the sensor exists to replace (§11.1, containment
        // 5). Refused here, where the error names the line, rather than
        // reported later as a saturation the owner has to decode.
        if setpoint.is_zero() {
            bail!("`{text}` is a setpoint of zero, which every reading would be past");
        }
        Ok(setpoint)
    }
}

/// `1h30m` → 5400 seconds. Tokens are a run of digits then one unit letter;
/// anything else is an error naming the grammar.
fn parse_duration(text: &str) -> Result<std::time::Duration> {
    let mut total: u64 = 0;
    let mut digits = String::new();
    let mut saw_token = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        if c.is_whitespace() && digits.is_empty() {
            continue;
        }
        let n: u64 = digits
            .parse()
            .map_err(|_| anyhow::anyhow!("`{text}` is not a duration like `24h` or `7d`"))?;
        digits.clear();
        let per = match c {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            'w' => 7 * 86_400,
            _ => bail!("`{text}` is not a duration — units are s, m, h, d, w"),
        };
        total = n
            .checked_mul(per)
            .and_then(|v| total.checked_add(v))
            .ok_or_else(|| anyhow::anyhow!("`{text}` is too long a duration"))?;
        saw_token = true;
    }
    if !digits.is_empty() || !saw_token {
        bail!("`{text}` is not a duration like `24h` or `7d` — every number needs a unit");
    }
    Ok(std::time::Duration::from_secs(total))
}

/// A sensor on a charter line, validated.
///
/// **The author rule is the same rule.** A sensor is typed by a person, at
/// any surface; the template shows one commented out and nothing else; no
/// model composes, suggests or tunes one. And its *reading* never reaches
/// the prompt (containment 2): the line's text already rides in the cached
/// prefix, the sensor's value is harness-only, and [`prompt_block`] does not
/// render this table at all.
#[derive(Debug, Clone, PartialEq)]
pub struct Sensor {
    pub kind: SensorKind,
    /// The setpoint in the kind's unit.
    pub setpoint: Setpoint,
    /// The setpoint as the owner spelled it — what a surface shows back and
    /// what the web editor writes on a save, so nothing the owner typed is
    /// re-rendered by the harness.
    pub setpoint_text: String,
}

impl Eq for Sensor {}

/// The owner's charter, in file order.
///
/// **Order is rank.** See the module doc — there is no field for it because
/// a second statement of priority disagrees with the first the moment either
/// is edited, the same rule `TASK-AGENT-DESIGN.md` R1 gives task urgency.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Charter {
    lines: Vec<CharterLine>,
}

/// The file's own shape: `[[line]]` tables, in the order they appear.
///
/// **Denies unknown top-level tables too** — `[[lines]]` (plural) or any
/// other typo'd name would otherwise vanish silently rather than parse as an
/// error, which is worse when it sits *beside* correctly-named lines: the
/// charter would load non-empty, `over_budget` would be false, and the
/// owner's ranking would be silently short whatever the typo'd entries were,
/// with nothing anywhere saying so.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCharter {
    #[serde(default, rename = "line")]
    line: Vec<RawLine>,
}

impl Charter {
    /// `~/.mecha/charter.toml` — the only path there is. See the module doc:
    /// this is deliberately not a `Config` field, because a configurable path
    /// is a path a project layer could set.
    pub fn default_path() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("charter.toml"))
    }

    /// Read and validate `path`. A missing file is an **empty** charter, not
    /// an error — a machine nobody has written one for yet must still start,
    /// on [`crate::skill::SkillStore::load`]'s rule for a missing directory.
    pub fn load(path: &Path) -> Result<Charter> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Charter::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Charter::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Parse and validate charter TOML that has not been written anywhere
    /// yet. The seam a surface that *accepts* an edit needs — the web
    /// settings page validates a proposed charter with exactly the reader
    /// every run will load it through, and refuses the save on an error,
    /// so a file that reaches disk is one that will load. `load` goes
    /// through here so the two can never diverge on what "valid" means.
    pub fn parse(text: &str) -> Result<Charter> {
        let raw: RawCharter = toml::from_str(text)?;
        Charter::validate(raw.line)
    }

    /// Only the conditions that make a line *ambiguous or unusable* refuse
    /// the whole document. Crossing [`CHARTER_CHAR_BUDGET`] is deliberately
    /// **not** one of them — see [`Charter::over_budget`] — on the
    /// `over_budget_domains` precedent for the learned-rules cap
    /// (`crate::learning`): a document that costs more of the cached prefix
    /// than argued still means exactly what it says, and dropping the whole
    /// charter because an eleventh line pushed it over a budget would un-rank
    /// every priority in it over a problem that is really about cost, not
    /// validity.
    fn validate(lines: Vec<RawLine>) -> Result<Charter> {
        let mut seen = BTreeSet::new();
        let mut kinds_seen: std::collections::BTreeMap<SensorKind, &str> = Default::default();
        for line in &lines {
            // Two lines with the same *kind* is not a tie rank can decide
            // (two kinds watching one store is — see `line_for_sensor`); it
            // is a dead line, because `line_for_sensor` finds the first and
            // the lower one can never be attributed anything. Refused naming
            // both, on the same principle as the duplicate id: an owner must
            // not write a sensor, believe it did something, and never find
            // out (found on review). The readings phase (`reading.rs`) did
            // not change this: two lines of one kind would read the same
            // store the same way, so the second still means nothing the
            // first does not.
            if let Some(sensor) = &line.sensor {
                if let Some(first) = kinds_seen.insert(sensor.kind, line.id.trim()) {
                    bail!(
                        "charter lines `{}` and `{}` both carry a `{}` sensor — only the \
                         higher-ranked line would ever be attributed anything, so the \
                         second does nothing; keep one",
                        first,
                        line.id.trim(),
                        sensor.kind.wire()
                    );
                }
            }
            if line.id.trim().is_empty() {
                bail!("a charter line has an empty `id`");
            }
            if line.text.trim().is_empty() {
                bail!("charter line `{}` has empty `text`", line.id);
            }
            // Trimmed, to match both the emptiness check two lines up and
            // `GoalRef::from_str` (`goal.rs`), which trims an id it parses —
            // `"x"` and `"x "` must collide here or they answer to the same
            // `charter:x` reference without this check ever having noticed.
            if !seen.insert(line.id.trim()) {
                // Ambiguous rather than merely untidy: a `GoalRef::Charter(id)`
                // naming a duplicated id would point at whichever line a
                // lookup happened to find first, silently.
                bail!(
                    "charter line id `{}` is used more than once — a goal reference \
                     naming it would not know which line it meant",
                    line.id
                );
            }
        }
        // Checking uniqueness on the trimmed form and then keeping the
        // untrimmed one would make the guarantee above a check rather than a
        // fact: `id = "x "` would still render with the trailing space and a
        // future `GoalRef::Charter("x")` lookup — itself trimmed, per
        // `goal.rs` — would miss a line recorded as `"x "`. Store what was
        // checked.
        //
        // The sensor's setpoint is typed here, against the kind beside it:
        // a setpoint the kind cannot read refuses the whole document, the
        // same fail-closed direction an unknown kind word already takes in
        // `serde`, and the error names the line so the owner knows which
        // table to fix.
        let lines = lines
            .into_iter()
            .map(|l| {
                let id = l.id.trim().to_string();
                let sensor = match l.sensor {
                    None => None,
                    Some(raw) => {
                        let setpoint_text = raw.setpoint.text();
                        let setpoint =
                            raw.kind.parse_setpoint(&setpoint_text).with_context(|| {
                                format!(
                                    "charter line `{id}`: sensor `{}` reads its setpoint as {}",
                                    raw.kind.wire(),
                                    raw.kind.unit().hint()
                                )
                            })?;
                        Some(Sensor {
                            kind: raw.kind,
                            setpoint,
                            setpoint_text,
                        })
                    }
                };
                Ok(CharterLine {
                    id,
                    text: l.text,
                    sensor,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Charter { lines })
    }

    pub fn lines(&self) -> &[CharterLine] {
        &self.lines
    }

    /// A charter from the file's own shape, validated — for a test in
    /// another module that wants a sensored line without writing TOML.
    #[cfg(test)]
    pub(crate) fn from_raw_lines(lines: Vec<RawLine>) -> Result<Charter> {
        Charter::validate(lines)
    }

    /// The highest-ranked line whose sensor watches one of `kinds`, or
    /// `None` when no line does.
    ///
    /// **Rank decides a tie**, which is the first consumer line order has
    /// ever had: two lines watching the same store by *different* kinds —
    /// one on how many drafts wait, one on how long — both bear on a
    /// released draft, and the one higher in the file is the one the owner
    /// ranked higher. Two lines with the *same* kind never reach here:
    /// `validate` refuses them, because the lower one would be a dead line.
    /// What this answers is *which line a record bearing on that store is
    /// attributed to*; it never reads the store itself.
    pub fn line_for_sensor(&self, kinds: &[SensorKind]) -> Option<&CharterLine> {
        self.lines
            .iter()
            .find(|l| l.sensor.as_ref().is_some_and(|s| kinds.contains(&s.kind)))
    }

    /// A line's rank: its index in file order, zero highest. `None` for an
    /// id the charter does not contain — a `serves: charter:<id>` is the
    /// model's own string, and a rank for a line that does not exist would
    /// be a rank for nothing. The second consumer line order has ever had
    /// (`line_for_sensor` was the first): §11.1's replay tiebreak, where a
    /// signed error against the top line replays before one against the
    /// fifth.
    pub fn rank_of(&self, id: &str) -> Option<usize> {
        let id = id.trim();
        self.lines.iter().position(|l| l.id == id)
    }

    /// Does any line carry a sensor? A surface that says "attributed by
    /// sensor" reads this first, so a charter with none is reported as
    /// having none rather than as attributing nothing.
    pub fn has_sensors(&self) -> bool {
        self.lines.iter().any(|l| l.sensor.is_some())
    }

    /// How many characters the charter actually costs when rendered into the
    /// system prompt — [`prompt_block`]'s own length, not just the authored
    /// `id`/`text` content. The header and the per-line `"N. `id` — "`
    /// formatting ride in the cached prefix too, so measuring only the
    /// authored text would under-report the true cost by a few hundred
    /// characters of fixed overhead. What [`Charter::over_budget`] checks
    /// against [`CHARTER_CHAR_BUDGET`], and the same number every message
    /// that quotes "characters" beside "the prompt" means.
    pub fn char_count(&self) -> usize {
        prompt_block(self).map_or(0, |b| b.chars().count())
    }

    /// Costs more of the cached prefix than [`CHARTER_CHAR_BUDGET`] argues
    /// for. Not refused by [`Charter::load`] — see its doc comment — so a
    /// caller that cares (today: `mecha doctor`) checks this after loading.
    pub fn over_budget(&self) -> bool {
        self.char_count() > CHARTER_CHAR_BUDGET
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// The closed set of sensor kinds as a surface offers them to the owner —
/// `[{kind, unit, hint, describe}]` in `SensorKind::ALL`'s order. Served by
/// the web settings endpoint so the form's select is this list and not a
/// copy that drifts; the hint is the parser's own unit sentence, and there
/// is deliberately no default kind and no default setpoint in it — the page
/// proposes nothing, the owner types both (§11.1's author rule).
pub fn sensor_kinds_json() -> serde_json::Value {
    serde_json::Value::Array(
        SensorKind::ALL
            .iter()
            .map(|k| {
                serde_json::json!({
                    "kind": k.wire(),
                    "unit": k.unit().wire(),
                    "hint": k.unit().hint(),
                    "describe": k.describe(),
                })
            })
            .collect(),
    )
}

/// The comments-only template a surface may write when no charter exists
/// yet, so the first edit never starts from an empty buffer — which is how
/// a first charter ends up shaped wrong. **No active `[[line]]` entries**:
/// a template that shipped priorities would be mecha authoring the charter,
/// the one thing every surface here refuses (§11), and the test on this
/// constant fails on any uncommented line. Lives here so the TUI's `e` and
/// the web settings editor hand out the same bytes rather than two copies
/// that drift. The commented example exists because the costliest authoring
/// mistake (§11) is a "never disappoint"-shaped line, and the place to say
/// so is inside the file being edited.
pub const TEMPLATE: &str = "\
# Your charter: standing priorities, in your own words, ranked highest
# first — ORDER IS RANK. There is no priority field; when two lines
# conflict, the higher one wins outright, and re-ranking is moving a line.
#
# mecha only ever reads this file. Each entry is:
#
#   [[line]]
#   id = \"a-short-stable-slug\"     # unique; goal references point at it
#   text = \"The priority itself, one or two sentences.\"
#
# One authoring trap, from the design doc: a line shaped like \"never
# disappoint anyone\" produces sycophancy and withheld bad news. Point it
# the other way — e.g.:
#
#   [[line]]
#   id = \"tell-the-truth-early\"
#   text = \"Tell me the truth early, especially when it disappoints.\"
#
# A line may carry a sensor: an observable mecha reads from its own stores,
# with a setpoint you wrote saying what the line means by \"short\" or
# \"few\". mecha then attributes a run that touched what the sensor watches
# to that line. Kinds (each fixes its setpoint's unit): outbox_waiting
# (count), outbox_age (duration), question_latency (duration),
# request_closure (duration), intervention_rate (rate, e.g. \"20%\"). The
# reading never enters a prompt.
#
#   [[line]]
#   id = \"answer-what-waits-on-me\"
#   text = \"Keep what waits on me short: a staged draft should not sit for days.\"
#   [line.sensor]
#   kind = \"outbox_age\"
#   setpoint = \"24h\"
";

/// The block rendered straight into the system prompt. `None` when the
/// charter is empty, so a machine with no charter authored yet sends no block
/// at all — the same reason [`crate::skill::prompt_block`] returns `None` on
/// an empty store.
///
/// The rendering [`Charter::char_count`] measures: with the `serves:` ask,
/// because `todo` is a default builtin and the surface without it is the
/// exception. See [`prompt_block_for`] for the switch.
pub fn prompt_block(charter: &Charter) -> Option<String> {
    prompt_block_for(charter, true)
}

/// [`prompt_block`], told whether the `todo` tool is in the run's surface.
///
/// **The block asks for a charter cite** (`docs/GOAL-SYSTEM-DESIGN.md`
/// §17.1's prerequisite): the plan's `serves:` is the one producer of a
/// goal reference an ordinary run has, and thirty days of corpus held zero
/// sessions naming one while this block deliberately did not ask. It asks
/// only when `todo` is registered — a system prompt saying "pass `serves`
/// on the `todo` tool" to a surface with no such tool (a narrow `--tool`
/// allowlist, Slack's own set) costs the model a turn on a call that can
/// only fail, the same reason `setup.rs` withholds the skills block when
/// `skill` is not in the surface. The list of lines renders either way.
///
/// What the sentence does not do: render a sensor. A sensored line is
/// listed exactly as an unsensored one — the sensor's kind and setpoint are
/// harness-only (§11.1, containment 2), and a number in the prompt is a
/// number the model reasons about.
pub fn prompt_block_for(charter: &Charter, todo_in_surface: bool) -> Option<String> {
    if charter.is_empty() {
        return None;
    }
    let mut out = String::from(
        "## Charter\n\n\
         Standing priorities the owner has written for you, ranked highest first \
         and listed in that order. They are not weighted: when two conflict, the \
         higher one wins outright, whatever the lower one would otherwise argue \
         for — no amount of urgency on a lower line outranks a higher one.\n\n",
    );
    for (i, line) in charter.lines().iter().enumerate() {
        out.push_str(&format!("{}. `{}` — {}\n", i + 1, line.id, line.text));
    }
    if todo_in_surface {
        out.push_str(
            "\nWhen you write a plan with the `todo` tool, say which line the work serves: \
             `serves: charter:<id>` with the line's id from this list, or `task:<id>` when \
             the work serves a task on the board. Name the one line it most serves, and \
             leave `serves` out when none applies.\n",
        );
    }
    Some(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template must never author a priority: a `[[line]]` not behind a
    /// comment would ship ranked content mecha wrote, which is the invariant
    /// every charter surface exists to refuse.
    #[test]
    fn the_template_carries_no_active_lines() {
        for l in TEMPLATE.lines() {
            let l = l.trim();
            assert!(
                l.is_empty() || l.starts_with('#'),
                "template has an uncommented line: {l:?}"
            );
        }
        // And it stays honest as a TOML document: parsing it yields nothing.
        let c = Charter::parse(TEMPLATE).unwrap();
        assert!(c.is_empty());
    }

    /// The file's shape, for `validate`.
    fn line(id: &str, text: &str) -> RawLine {
        RawLine {
            id: id.to_string(),
            text: text.to_string(),
            sensor: None,
        }
    }

    /// The validated shape, for a `Charter` built by hand.
    fn cline(id: &str, text: &str) -> CharterLine {
        CharterLine {
            id: id.to_string(),
            text: text.to_string(),
            sensor: None,
        }
    }

    fn sensored(id: &str, text: &str, kind: SensorKind, setpoint: &str) -> RawLine {
        RawLine {
            sensor: Some(RawSensor {
                kind,
                setpoint: RawSetpoint::Text(setpoint.to_string()),
            }),
            ..line(id, text)
        }
    }

    /// Write `raw` to a scratch `charter.toml` and load it, unique per test
    /// and thread so parallel tests don't collide on the path.
    fn write_and_load(raw: &str) -> Result<Charter> {
        let dir = std::env::temp_dir().join(format!(
            "mecha-charter-test-{}-{:?}-{:?}",
            std::process::id(),
            std::thread::current().id(),
            std::time::Instant::now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("charter.toml");
        std::fs::write(&path, raw).unwrap();
        Charter::load(&path)
    }

    #[test]
    fn the_standard_shape_parses_in_file_order() {
        let raw = r#"
[[line]]
id = "protect-the-owner"
text = "Protect the owner's interests above all else."

[[line]]
id = "tell-the-truth-early"
text = "Tell the owner the truth early, especially when it disappoints."
"#;
        let charter = write_and_load(raw).unwrap();
        assert_eq!(
            charter.lines(),
            &[
                cline(
                    "protect-the-owner",
                    "Protect the owner's interests above all else."
                ),
                cline(
                    "tell-the-truth-early",
                    "Tell the owner the truth early, especially when it disappoints."
                ),
            ]
        );
    }

    // --- sensored lines (§11.1) ---

    #[test]
    fn a_sensored_line_parses_with_its_setpoint_typed_by_the_kind() {
        let raw = r#"
[[line]]
id = "answer-what-waits-on-me"
text = "Keep what waits on me short."
[line.sensor]
kind = "outbox_age"
setpoint = "24h"

[[line]]
id = "few-open-drafts"
text = "Few drafts at once."
[line.sensor]
kind = "outbox_waiting"
setpoint = 3
"#;
        let charter = write_and_load(raw).unwrap();
        let s = charter.lines()[0].sensor.as_ref().unwrap();
        assert_eq!(s.kind, SensorKind::OutboxAge);
        assert_eq!(
            s.setpoint,
            Setpoint::Duration(std::time::Duration::from_secs(24 * 3600))
        );
        assert_eq!(s.setpoint_text, "24h", "the owner's spelling survives");
        // A bare number is how a count is naturally written; it is kept as
        // its own spelling so a web round trip changes quoting and nothing
        // else.
        let s = charter.lines()[1].sensor.as_ref().unwrap();
        assert_eq!(s.setpoint, Setpoint::Count(3));
        assert_eq!(s.setpoint_text, "3");
        assert!(charter.has_sensors());
    }

    /// The kind set is closed at the type: a word this binary has not heard
    /// of is a load error, never a line that silently watches nothing — and
    /// on an older binary `setup` runs un-chartered with a stderr line and
    /// `mecha doctor` reports it — see `SensorKind`'s doc for why that is
    /// not the refusal §11.1 claims.
    #[test]
    fn an_unknown_sensor_kind_is_a_load_error() {
        let raw = r#"
[[line]]
id = "a"
text = "one"
[line.sensor]
kind = "vibes"
setpoint = "high"
"#;
        let e = write_and_load(raw).unwrap_err().to_string();
        assert!(e.contains("parsing"), "{e}");
    }

    /// Each kind fixes its unit, and a setpoint the kind cannot read refuses
    /// the document naming the line and the unit — the place to catch an
    /// hour the owner meant as a day is the parse, not a saturated reading
    /// weeks later (containment 5).
    #[test]
    fn a_setpoint_in_the_wrong_unit_is_refused_naming_the_line_and_the_unit() {
        let e =
            Charter::validate(vec![sensored("a", "one", SensorKind::OutboxAge, "3")]).unwrap_err();
        let e = format!("{e:#}");
        assert!(e.contains("line `a`"), "{e}");
        assert!(e.contains("duration"), "{e}");
        assert!(e.contains("every number needs a unit"), "{e}");

        let e = Charter::validate(vec![sensored("b", "two", SensorKind::OutboxWaiting, "24h")])
            .unwrap_err();
        assert!(format!("{e:#}").contains("whole number"), "{e:#}");

        let e = Charter::validate(vec![sensored(
            "c",
            "three",
            SensorKind::InterventionRate,
            "150%",
        )])
        .unwrap_err();
        assert!(format!("{e:#}").contains("between 0 and 1"), "{e:#}");
    }

    /// A setpoint nothing could ever be within reads as past on every run —
    /// the constant the sensor exists to replace (§11.1, containment 5) —
    /// so it is refused where the error names the line, in every unit.
    #[test]
    fn a_zero_setpoint_is_refused_in_every_unit() {
        for (kind, zero) in [
            (SensorKind::OutboxWaiting, "0"),
            (SensorKind::OutboxAge, "0s"),
            (SensorKind::QuestionLatency, "0h"),
            (SensorKind::InterventionRate, "0"),
            (SensorKind::InterventionRate, "0%"),
        ] {
            let err = kind.parse_setpoint(zero).unwrap_err().to_string();
            assert!(err.contains("setpoint of zero"), "{kind:?} {zero}: {err}");
        }
        let raw = r#"
[[line]]
id = "waits"
text = "Keep what waits on me short."
[line.sensor]
kind = "outbox_waiting"
setpoint = 0
"#;
        let err = format!("{:#}", Charter::parse(raw).unwrap_err());
        assert!(err.contains("charter line `waits`"), "{err}");
        assert!(err.contains("setpoint of zero"), "{err}");
    }

    /// The served list, as JSON, for the far side of the boundary: the
    /// docs demo's `fixtures.js` carries a hand copy of what
    /// `sensor_kinds_json` serves, and `website/scripts/check-charter-toml.mjs`
    /// reads this literal out of the source and asserts the fixture equals
    /// it, the way it pins the serialiser against `WEB_EDITOR_SAMPLE`. The
    /// test below asserts the literal equals the function, so a kind that
    /// joins or a hint that is reworded fails here first and the demo
    /// second — never neither (found on review).
    // sensor-kinds:begin
    const SENSOR_KINDS_JSON: &str = r#"[
  {"kind":"outbox_waiting","unit":"count","hint":"a whole number","describe":"how many outbox drafts wait on you"},
  {"kind":"outbox_age","unit":"duration","hint":"a duration like `24h` or `7d`","describe":"how long a staged draft has sat unreviewed"},
  {"kind":"question_latency","unit":"duration","hint":"a duration like `24h` or `7d`","describe":"how long a parked question waits for your answer"},
  {"kind":"request_closure","unit":"duration","hint":"a duration like `24h` or `7d`","describe":"how long a front-door request stays open"},
  {"kind":"intervention_rate","unit":"rate","hint":"a rate like `0.2` or `20%`","describe":"the share of recent runs you stepped into"}
]"#;
    // sensor-kinds:end

    #[test]
    fn the_marked_kinds_literal_is_what_the_server_serves() {
        let pinned: serde_json::Value = serde_json::from_str(SENSOR_KINDS_JSON).unwrap();
        assert_eq!(
            pinned,
            sensor_kinds_json(),
            "update the sensor-kinds literal (and the demo fixture) with the served list"
        );
    }

    /// The list a form offers is every kind, each with its unit's own hint
    /// — the sentence the parser's refusal uses — and no value: a default
    /// in this list would be the page proposing a number.
    #[test]
    fn the_kinds_a_form_offers_are_every_kind_with_the_parsers_own_hint_and_no_default() {
        let kinds = sensor_kinds_json();
        let arr = kinds.as_array().unwrap();
        assert_eq!(arr.len(), SensorKind::ALL.len());
        for (v, k) in arr.iter().zip(SensorKind::ALL) {
            assert_eq!(v["kind"], k.wire());
            assert_eq!(v["unit"], k.unit().wire());
            assert_eq!(v["hint"], k.unit().hint());
            assert!(v.get("setpoint").is_none() && v.get("default").is_none());
            // The refusal names the same hint the form shows.
            let err = format!("{:#}", Charter::parse(&format!(
                "[[line]]\nid = \"x\"\ntext = \"t\"\n[line.sensor]\nkind = \"{}\"\nsetpoint = \"nonsense\"\n",
                k.wire()
            )).unwrap_err());
            assert!(err.contains(k.unit().hint()), "{err}");
        }
    }

    #[test]
    fn every_setpoint_unit_has_a_grammar() {
        assert_eq!(
            SensorKind::QuestionLatency.parse_setpoint("1h30m").unwrap(),
            Setpoint::Duration(std::time::Duration::from_secs(5400))
        );
        assert_eq!(
            SensorKind::RequestClosure.parse_setpoint("1w").unwrap(),
            Setpoint::Duration(std::time::Duration::from_secs(7 * 86_400))
        );
        assert_eq!(
            SensorKind::OutboxWaiting.parse_setpoint("3").unwrap(),
            Setpoint::Count(3)
        );
        assert_eq!(
            SensorKind::InterventionRate.parse_setpoint("20%").unwrap(),
            Setpoint::Rate(0.2)
        );
        assert_eq!(
            SensorKind::InterventionRate.parse_setpoint("0.2").unwrap(),
            Setpoint::Rate(0.2)
        );
        assert!(SensorKind::OutboxWaiting.parse_setpoint("-1").is_err());
        assert!(SensorKind::OutboxAge.parse_setpoint("24 hours").is_err());
        assert!(SensorKind::OutboxAge.parse_setpoint("").is_err());
        // Every kind answers `unit`, and the two lists agree on length — a
        // kind added to the enum without a row here is a compile error in
        // `unit`'s match, and one added to `ALL` twice is caught below.
        let mut seen = std::collections::BTreeSet::new();
        for k in SensorKind::ALL {
            assert!(
                seen.insert(k.wire()),
                "{k:?} appears twice in SensorKind::ALL"
            );
            let _ = k.unit();
        }
    }

    /// Two lines carrying the same kind: the lower one could never be
    /// attributed anything, so the document is refused naming both — a
    /// dead sensor is the "wrote it, believed it, never found out" failure
    /// one field down from a stray key.
    #[test]
    fn two_lines_with_the_same_sensor_kind_are_refused_naming_both() {
        let e = Charter::validate(vec![
            sensored("first", "one", SensorKind::OutboxAge, "24h"),
            line("plain", "two"),
            sensored("second", "three", SensorKind::OutboxAge, "48h"),
        ])
        .unwrap_err()
        .to_string();
        assert!(e.contains("`first`") && e.contains("`second`"), "{e}");
        assert!(e.contains("outbox_age"), "{e}");
        // Different kinds on one store remain a tie rank decides.
        assert!(Charter::validate(vec![
            sensored("a", "one", SensorKind::OutboxAge, "24h"),
            sensored("b", "two", SensorKind::OutboxWaiting, "3"),
        ])
        .is_ok());
    }

    /// A stray key under the sensor table is refused like one under the
    /// line — `threshold = ` where `setpoint = ` was meant must not parse as
    /// a sensor with no setpoint.
    #[test]
    fn a_stray_field_on_a_sensor_is_a_load_error() {
        let raw = r#"
[[line]]
id = "a"
text = "one"
[line.sensor]
kind = "outbox_age"
setpoint = "24h"
weight = 2
"#;
        assert!(write_and_load(raw).is_err());
    }

    /// The sensor's reading is harness-only (containment 2): the block lists
    /// a sensored line exactly as an unsensored one, so neither the kind
    /// word nor the setpoint ever rides in a prompt.
    #[test]
    fn the_prompt_block_never_renders_a_sensor() {
        let charter = Charter::validate(vec![sensored(
            "answer-what-waits",
            "Keep what waits on me short.",
            SensorKind::OutboxAge,
            "24h",
        )])
        .unwrap();
        let block = prompt_block(&charter).unwrap();
        assert!(block.contains("answer-what-waits"));
        assert!(!block.contains("24h"), "{block}");
        assert!(!block.contains("outbox_age"), "{block}");
        assert!(!block.contains("sensor"), "{block}");
    }

    /// Rank is the tiebreak, and this is its first consumer: two lines
    /// watching the outbox both bear on a draft, and the higher one wins.
    #[test]
    fn line_for_sensor_returns_the_highest_ranked_match() {
        let charter = Charter::validate(vec![
            line("unsensored", "first"),
            sensored("count", "few", SensorKind::OutboxWaiting, "3"),
            sensored("age", "short", SensorKind::OutboxAge, "24h"),
            sensored("asks", "answered", SensorKind::QuestionLatency, "1d"),
        ])
        .unwrap();
        let both = [SensorKind::OutboxAge, SensorKind::OutboxWaiting];
        assert_eq!(charter.line_for_sensor(&both).unwrap().id, "count");
        assert_eq!(
            charter
                .line_for_sensor(&[SensorKind::OutboxAge])
                .unwrap()
                .id,
            "age"
        );
        assert_eq!(
            charter
                .line_for_sensor(&[SensorKind::QuestionLatency])
                .unwrap()
                .id,
            "asks"
        );
        assert!(charter
            .line_for_sensor(&[SensorKind::RequestClosure])
            .is_none());
        assert!(Charter::default().line_for_sensor(&both).is_none());
        assert!(!Charter::default().has_sensors());
    }

    /// The block asks for a charter cite (§17.1's prerequisite) only where
    /// the `todo` tool exists to carry it — asking a surface with no such
    /// tool costs a turn on a call that can only fail.
    #[test]
    fn the_block_asks_for_a_serves_cite_only_when_todo_is_in_the_surface() {
        let charter = Charter::validate(vec![line("a", "one")]).unwrap();
        let with = prompt_block_for(&charter, true).unwrap();
        assert!(with.contains("serves: charter:<id>"), "{with}");
        assert!(with.contains("`todo`"), "{with}");
        let without = prompt_block_for(&charter, false).unwrap();
        assert!(!without.contains("serves"), "{without}");
        assert!(without.contains("`a` — one"), "the lines render either way");
        // `prompt_block` is the with-cite rendering, which is what
        // `char_count` measures.
        assert_eq!(prompt_block(&charter), Some(with));
    }

    #[test]
    fn a_missing_file_is_an_empty_charter_not_an_error() {
        let path = std::env::temp_dir().join("mecha-charter-does-not-exist.toml");
        let _ = std::fs::remove_file(&path);
        let charter = Charter::load(&path).unwrap();
        assert!(charter.is_empty());
    }

    #[test]
    fn a_typo_d_table_name_is_a_load_error_not_a_silently_short_charter() {
        // `[[lines]]` (plural) beside a correctly-named `[[line]]` used to
        // vanish rather than fail — non-empty, under budget, and quietly
        // missing whatever the typo'd entries were meant to say.
        let raw = r#"
[[line]]
id = "protect-the-owner"
text = "Protect the owner's interests above all else."

[[lines]]
id = "tell-the-truth-early"
text = "Tell the owner the truth early, especially when it disappoints."
"#;
        let e = write_and_load(raw).unwrap_err().to_string();
        assert!(e.contains("parsing"), "{e}");
    }

    #[test]
    fn a_stray_priority_field_on_a_line_is_a_load_error() {
        // §11: rank is file order and there is deliberately no priority
        // field. Accepting one silently would let an owner write it and
        // believe it did something.
        let raw = r#"
[[line]]
id = "a"
text = "one"
priority = 1
"#;
        assert!(write_and_load(raw).is_err());
    }

    #[test]
    fn a_duplicate_id_is_refused_because_a_reference_to_it_would_be_ambiguous() {
        let e = Charter::validate(vec![line("a", "one"), line("a", "two")])
            .unwrap_err()
            .to_string();
        assert!(e.contains("used more than once"), "{e}");
    }

    #[test]
    fn ids_differing_only_by_surrounding_whitespace_still_collide() {
        // `GoalRef::from_str` trims an id it parses (`goal.rs`), so `"x"` and
        // `"x "` answer to the same `charter:x` reference — this check has to
        // trim too, or two visually distinct-looking lines pass as unique
        // and then can't be told apart by anything that resolves the id.
        let e = Charter::validate(vec![line("a", "one"), line("a ", "two")])
            .unwrap_err()
            .to_string();
        assert!(e.contains("used more than once"), "{e}");
    }

    #[test]
    fn a_surviving_id_is_stored_trimmed_not_just_checked_trimmed() {
        // Checking uniqueness on the trimmed form and then keeping the
        // untrimmed one would make the guarantee a check rather than a
        // fact — a lone `"x "` line would pass validation and then render
        // with the trailing space, answering to `charter:x` by nothing more
        // than luck.
        let charter = Charter::validate(vec![line(" x ", "one")]).unwrap();
        assert_eq!(charter.lines()[0].id, "x");
    }

    #[test]
    fn char_count_is_the_rendered_costs_not_just_the_authored_text() {
        // The header and the `"1. `id` — "` formatting ride in the cached
        // prefix too — a budget checked only against authored text would
        // under-report the true cost, and the messages that quote this
        // number beside "in the prompt" would be naming a different number
        // than the one that actually rides there.
        let charter = Charter::validate(vec![line("a", "short")]).unwrap();
        assert_eq!(
            charter.char_count(),
            prompt_block(&charter).unwrap().chars().count()
        );
        assert!(charter.char_count() > "a".len() + "short".len());
    }

    #[test]
    fn an_empty_id_or_text_is_refused() {
        assert!(Charter::validate(vec![line("", "text")]).is_err());
        assert!(Charter::validate(vec![line("id", "  ")]).is_err());
    }

    #[test]
    fn a_charter_over_the_character_budget_still_loads_and_says_so() {
        // Refusing the whole document over its eleventh line would un-rank
        // every priority in it for a problem that is about cost, not
        // validity — the `over_budget_domains` precedent, applied here.
        let long = "x".repeat(CHARTER_CHAR_BUDGET + 1);
        let charter = Charter::validate(vec![line("only-line", &long)]).unwrap();
        assert_eq!(charter.lines().len(), 1);
        assert!(charter.over_budget());
    }

    #[test]
    fn a_charter_under_the_budget_is_not_over_it() {
        let charter = Charter::validate(vec![line("a", "short")]).unwrap();
        assert!(!charter.over_budget());
    }

    #[test]
    fn an_empty_charter_contributes_no_block_at_all() {
        assert_eq!(prompt_block(&Charter::default()), None);
    }

    #[test]
    fn the_block_lists_lines_in_file_order_not_sorted() {
        // Unlike skills, order is content: a charter authored `["b-line",
        // "a-line"]` must render in that order, never alphabetically.
        let charter = Charter {
            lines: vec![
                cline("b-line", "second priority"),
                cline("a-line", "first priority"),
            ],
        };
        let block = prompt_block(&charter).unwrap();
        let b = block.find("b-line").unwrap();
        let a = block.find("a-line").unwrap();
        assert!(b < a, "{block}");
    }

    #[test]
    fn the_block_explains_the_ordering_is_load_bearing() {
        let charter = Charter {
            lines: vec![cline("only", "the only priority")],
        };
        let block = prompt_block(&charter).unwrap();
        assert!(block.contains("not weighted"), "{block}");
    }
    /// The exact bytes the web settings page writes, verbatim.
    ///
    /// **This literal is a shared fixture, not a copy.** The Svelte
    /// serialiser (`web/src/lib/charter-toml.js`) must produce these bytes
    /// and this reader must load them, and neither half proves the agreement
    /// alone: a hand-copied expectation here stays green through any
    /// regression in `esc` or `serialize`. So
    /// `website/scripts/check-charter-toml.mjs` reads *this* string out of
    /// *this* file and asserts the serialiser emits it byte-for-byte, which
    /// is what makes an edit to either side fail the other. Keep the markers
    /// intact; that script finds the literal by them.
    ///
    /// The editor keeps everything above the first `[[line]]` untouched (the
    /// owner's header comments, and the whole template on a first charter)
    /// and regenerates only the tables, always as single-line basic strings,
    /// because an escape sequence is unambiguous where a bare quote or
    /// newline is not. A line's sensor is written back as a `[line.sensor]`
    /// table with the owner's own setpoint spelling — the editor does not
    /// compose or edit one, it carries one through a save, which is the
    /// half of §11.1's "parser, serialiser and template move together" that
    /// this fixture pins: a serialiser that dropped the table would silently
    /// delete the owner's sensor on the next re-rank.
    // web-editor-sample:begin
    const WEB_EDITOR_SAMPLE: &str = r#"# What mecha is for, most important first.
#
# Order is rank.

[[line]]
id = "say-no-early"
text = "A refusal on Monday is a kindness."

[[line]]
id = "quote-and-break"
text = "She said \"no\" early.\nAnd meant it."

[[line]]
id = "answer-what-waits"
text = "Keep what waits on me short."
[line.sensor]
kind = "outbox_age"
setpoint = "24h"
"#;
    // web-editor-sample:end

    /// The order of the tables *is* the ranking — the editor's drag gesture
    /// writes nothing else — so this asserts file order, not membership, and
    /// that the serialiser's escaping survives the reader.
    #[test]
    fn the_web_editors_serialisation_is_what_this_reader_loads() {
        let charter = Charter::parse(WEB_EDITOR_SAMPLE).unwrap();
        let ids: Vec<&str> = charter.lines().iter().map(|l| l.id.as_str()).collect();
        assert_eq!(
            ids,
            ["say-no-early", "quote-and-break", "answer-what-waits"],
            "file order is rank"
        );
        assert_eq!(
            charter.lines()[1].text,
            "She said \"no\" early.\nAnd meant it.",
            "the editor's escaping must survive the reader"
        );
        assert_eq!(charter.lines()[0].sensor, None);
        let s = charter.lines()[2].sensor.as_ref().unwrap();
        assert_eq!(
            (s.kind, s.setpoint_text.as_str()),
            (SensorKind::OutboxAge, "24h")
        );
    }
}
