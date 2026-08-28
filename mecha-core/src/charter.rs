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
//! edit its way around every other guardrail, so there is deliberately no
//! write path here at all: the owner edits `charter.toml` with a text editor,
//! and this module only ever reads it.
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
/// **Denies unknown fields**, unlike [`crate::skill::Skill`]'s frontmatter —
/// that leniency is for portability across harnesses that might author a
/// `SKILL.md`, and nothing else authors a `charter.toml`. A stray `priority`
/// or `rank` key is exactly the field §11 says there deliberately is none of;
/// silently dropping it would let an owner write one, believe it did
/// something, and never find out it didn't.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharterLine {
    /// What a [`crate::goal::GoalRef::Charter`] names. Unique within a
    /// charter — see [`Charter::load`].
    pub id: String,
    pub text: String,
}

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
    line: Vec<CharterLine>,
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
        let raw: RawCharter =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
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
    fn validate(lines: Vec<CharterLine>) -> Result<Charter> {
        let mut seen = BTreeSet::new();
        for line in &lines {
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
        Ok(Charter { lines })
    }

    pub fn lines(&self) -> &[CharterLine] {
        &self.lines
    }

    /// Total characters across every id and text — what
    /// [`Charter::over_budget`] checks against [`CHARTER_CHAR_BUDGET`].
    pub fn char_count(&self) -> usize {
        self.lines
            .iter()
            .map(|l| l.id.chars().count() + l.text.chars().count())
            .sum()
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

/// The block rendered straight into the system prompt. `None` when the
/// charter is empty, so a machine with no charter authored yet sends no block
/// at all — the same reason [`crate::skill::prompt_block`] returns `None` on
/// an empty store.
pub fn prompt_block(charter: &Charter) -> Option<String> {
    if charter.is_empty() {
        return None;
    }
    // No instruction to cite a line's id via `serves`, deliberately: `todo`'s
    // schema documents only `task:<id>` there (`tool/todo.rs`), and this
    // block is unconditional — rendered whether or not `todo` is even in the
    // tool surface (a narrow `--tool` allowlist, Slack's own set). Asking for
    // a citation with nowhere reliable to put it, or a tool that may not
    // exist, is worse than not asking; wiring that up is the appraisal
    // consumer's job (see the rung 10 note in `GOAL-SYSTEM-DESIGN.md`), not
    // this block's.
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
    Some(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, text: &str) -> CharterLine {
        CharterLine {
            id: id.to_string(),
            text: text.to_string(),
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
                line(
                    "protect-the-owner",
                    "Protect the owner's interests above all else."
                ),
                line(
                    "tell-the-truth-early",
                    "Tell the owner the truth early, especially when it disappoints."
                ),
            ]
        );
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
                line("b-line", "second priority"),
                line("a-line", "first priority"),
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
            lines: vec![line("only", "the only priority")],
        };
        let block = prompt_block(&charter).unwrap();
        assert!(block.contains("not weighted"), "{block}");
    }
}
