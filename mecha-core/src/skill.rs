//! Skills: named procedures the user writes and the model loads on demand.
//!
//! A skill is a directory holding a `SKILL.md` — frontmatter naming it and
//! saying when to use it, then a markdown body that *is* the procedure. The
//! shape is the Agent Skills standard, and the reason to take a standard here
//! rather than invent a format is that the procedures worth writing are
//! portable: this repository already carries two of them, written for the
//! other side of it.
//!
//! ## Progressive disclosure is the whole point
//!
//! | Level | Loaded | Cost |
//! |---|---|---|
//! | 1 · metadata | always, in the system prompt | ~100 tokens per skill |
//! | 2 · body | when the model calls `skill` | the body, once |
//! | 3 · bundled files | when the body points at one | nothing until read |
//!
//! So a mailbox full of skills costs almost nothing until one is relevant,
//! which is what makes this the pressure valve for the learned-rule cap:
//! `MAX_ACTIVE_RULES_PER_DOMAIN` is small because the always-on prefix is
//! finite, and a procedure like *how to answer a rec-letter request* is too
//! long for a rule, too specific to be worth a slot, and irrelevant on almost
//! every run. Skills do not loosen that cap — they make it affordable.
//!
//! ## Why this is allowed to be liberal where learning is strict
//!
//! **A skill is user-authored, and there is deliberately no way for it not to
//! be.** No `mecha skill install`, no registry client, no remote body, and
//! nothing here is ever written by a model or derived from a session. That is
//! the whole safety argument, and it is why loading a skill arms no taint: a
//! skill body is the user's own words, exactly like the system prompt and the
//! `*.user.toml` rules, and treating it as third-party content would be a
//! category error in the direction that makes the model invent explanations
//! for its own harness.
//!
//! The absence of an install verb is the feature rather than an omission.
//! Snyk scanned 3,984 published skills and found 36.8% carrying at least one
//! security flaw, 13.4% a critical one, and 76 confirmed malicious payloads —
//! and Datadog's finding is the sharper one for a harness: *a cloned
//! repository can bring skills into a trusted session even if the developer
//! never installed one from a marketplace*. mecha already refuses that shape
//! for triggers, in writing. It refuses it here for the same reason: the
//! store is **global only**, and a project's `mecha.toml` may narrow the set
//! by name but can never author a skill or add one. See
//! [`crate::config::SkillsConfig`].
//!
//! ## The frontmatter is YAML, and that is not ours to change
//!
//! Every other file mecha reads is TOML, and this one is not, because the
//! Agent Skills standard fixes YAML and roughly forty implementations read it.
//! A skill written here should load in any of them and one written for any of
//! them should load here; inventing a dialect would spend that for internal
//! consistency, which is the trade `docs/SKILLS-RESEARCH.md` §9 lists under
//! *what not to build*.
//!
//! Unknown keys are **ignored rather than refused**, for the same reason: a
//! skill carrying a field some other harness understands must not fail to
//! load here. What is refused is a key mecha knows and cannot use — a
//! `description` that is a list, a `tools` that is empty — because that is an
//! authoring mistake rather than a portability one.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// `name`'s constraints, from the standard.
///
/// The vendor-name exclusion is theirs and is kept rather than dropped: a
/// skill called `claude-notes` authored for mecha would stop loading the day
/// somebody moved it to the harness it was named after, which is exactly the
/// portability this format is being adopted for.
const MAX_NAME: usize = 64;
const MAX_DESCRIPTION: usize = 1024;

/// One skill, as read off the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    /// What it does *and when to use it* — this carries the entire discovery
    /// burden, because it is all the model sees until it loads the body.
    ///
    /// The same sentence [`crate::subagent::SubagentProfile`] already carries:
    /// say when to use it, not just what it is. Two independent designs
    /// arriving at the same instruction is a good sign it is load-bearing.
    pub description: String,
    /// Optional keywords, a cheap deterministic complement to the model
    /// inferring relevance from prose. Costs nothing at level 1 because they
    /// ride in the same line the description already needed.
    pub triggers: Vec<String>,
    /// If present, the tool surface this skill narrows to while loaded.
    ///
    /// Narrow only, never widen — the capability-override rule in a second
    /// setting. Enforcement is not here: it is
    /// [`crate::tool::Tool::narrows_surface_to`], so the loop learns that
    /// some tool may restrict the surface and never that skills exist.
    pub tools: Option<Vec<String>>,
    /// The procedure. Reproduced verbatim when loaded — a paraphrased
    /// procedure is a different procedure.
    pub body: String,
    /// Where it lives, so a body may point at a file beside it.
    pub dir: PathBuf,
}

impl Skill {
    /// The level-1 line: everything the model knows before it loads anything.
    pub fn summary_line(&self) -> String {
        let mut line = format!("- `{}` — {}", self.name, self.description);
        if !self.triggers.is_empty() {
            line.push_str(&format!(" (keywords: {})", self.triggers.join(", ")));
        }
        line
    }

    /// Read and validate one `<dir>/SKILL.md`.
    pub fn load(dir: &Path) -> Result<Skill> {
        let path = dir.join("SKILL.md");
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut skill = Skill::parse(&raw, dir)?;
        skill.dir = dir.to_path_buf();

        // The directory name is how a person finds a skill and how the model
        // names it; a mismatch means one of the two is a lie. Refused rather
        // than resolved in either direction, because guessing which the author
        // meant is how a rename half-lands.
        let folder = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if folder != skill.name {
            bail!(
                "{}: frontmatter says `name = {}` but the directory is `{folder}` — \
                 they have to match, since the directory is how the skill is found \
                 and the name is how it is called",
                path.display(),
                skill.name
            );
        }
        Ok(skill)
    }

    /// Split frontmatter from body, parse it, and validate.
    pub fn parse(raw: &str, dir: &Path) -> Result<Skill> {
        let (fm, body) = split_frontmatter(raw)?;
        // Budgets rather than defaults: this parser reads files that may have
        // arrived with a repository, and an unbounded one is a denial of
        // service against a startup path. Frontmatter is a handful of short
        // lines, so the ceilings are far above anything honest.
        let options = serde_saphyr::options!(
            // A repeated key is an authoring mistake with two plausible
            // readings, and silently taking one of them is how a skill ends
            // up doing something its author did not write.
            duplicate_keys: serde_saphyr::DuplicateKeyPolicy::Error,
            budget: serde_saphyr::budget!(max_depth: 8, max_documents: 1)
        );
        let fm: Frontmatter = serde_saphyr::from_str_with_options(&fm, options)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("parsing the YAML frontmatter")?;

        validate_name(&fm.name)?;
        validate_description(&fm.description)?;

        if fm.tools.as_ref().is_some_and(|t| t.is_empty()) {
            // An empty list reads as "no tools at all", which would strand the
            // run; an absent key is what "do not narrow" is spelled as. The
            // difference is invisible enough to be worth refusing.
            bail!(
                "`tools` is present but empty — omit the key to leave the surface \
                 alone, or name the tools this skill needs"
            );
        }

        Ok(Skill {
            name: fm.name,
            description: fm.description,
            triggers: fm.triggers.unwrap_or_default(),
            tools: fm.tools,
            body: body.trim().to_string(),
            dir: dir.to_path_buf(),
        })
    }
}

/// The frontmatter fields mecha uses.
///
/// Not `deny_unknown_fields`, deliberately: a skill written for another
/// harness may carry keys this one has never heard of, and refusing it would
/// give up the portability that is the whole argument for the format.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    /// Optional keywords. `Option` rather than `#[serde(default)]` so an
    /// explicitly empty list stays distinguishable from an absent key, which
    /// `tools` needs and this shares for symmetry.
    triggers: Option<Vec<String>>,
    tools: Option<Vec<String>>,
}

/// Frontmatter and body.
///
/// `---` opens it and `---` closes it, per the standard. The body is
/// everything after, kept as written.
fn split_frontmatter(raw: &str) -> Result<(String, String)> {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        bail!("no frontmatter — a SKILL.md opens with a `---` line");
    }

    let mut fm = String::new();
    let mut body = String::new();
    let mut closed = false;
    for line in lines {
        if !closed && line.trim_end() == "---" {
            closed = true;
            continue;
        }
        if closed {
            body.push_str(line);
            body.push('\n');
        } else {
            fm.push_str(line);
            fm.push('\n');
        }
    }
    if !closed {
        bail!("frontmatter opened with `---` and was never closed");
    }
    Ok((fm, body))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("`name` is empty");
    }
    if name.chars().count() > MAX_NAME {
        bail!("`name` is longer than {MAX_NAME} characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("`name` may hold only lowercase letters, digits and hyphens: got `{name}`");
    }
    let lower = name.to_ascii_lowercase();
    if lower.contains("anthropic") || lower.contains("claude") {
        bail!("`name` may not contain a vendor name (`{name}`) — the standard reserves those");
    }
    Ok(())
}

fn validate_description(description: &str) -> Result<()> {
    if description.trim().is_empty() {
        bail!("`description` is empty — it is the only thing the model sees before loading");
    }
    if description.chars().count() > MAX_DESCRIPTION {
        bail!("`description` is longer than {MAX_DESCRIPTION} characters");
    }
    // Angle brackets in the one field that rides in every prompt are refused
    // outright: the standard forbids XML tags there, and the reason is that
    // the block is assembled into a prompt where a closing tag could end a
    // section the harness opened.
    if description.contains('<') || description.contains('>') {
        bail!("`description` may not contain `<` or `>`");
    }
    Ok(())
}

/// Every skill on the machine, in a stable order.
#[derive(Debug, Clone, Default)]
pub struct SkillStore {
    /// Sorted by name. **Sorted rather than in directory order**, because the
    /// level-1 block rides at the front of the cached prefix and filesystem
    /// order is not an order — the same reason the tool registry is a
    /// `BTreeMap`.
    skills: Vec<Skill>,
}

/// A skill directory that would not load, kept so startup can say so.
///
/// A skill that silently fails to load looks exactly like a skill the model
/// chose not to use, which is the shape of the unrouted-domain warning and is
/// reported for the same reason.
#[derive(Debug, Clone)]
pub struct SkillError {
    pub dir: PathBuf,
    pub why: String,
}

impl SkillStore {
    /// `~/.mecha/skills`.
    pub fn default_dir() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("skills"))
    }

    /// Read every `<dir>/*/SKILL.md`.
    ///
    /// Best-effort per skill, like every other reader over a store here: one
    /// unparseable skill is a finding, not a crash, and never suppresses the
    /// ones beside it. **Read-only** — a missing directory is an empty store,
    /// because an agent that has been given no skills must not create state by
    /// starting.
    pub fn load(dir: &Path) -> (SkillStore, Vec<SkillError>) {
        let mut skills = Vec::new();
        let mut errors = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return (SkillStore::default(), errors);
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if !path.join("SKILL.md").is_file() {
                continue;
            }
            match Skill::load(&path) {
                Ok(skill) => skills.push(skill),
                Err(e) => errors.push(SkillError {
                    dir: path,
                    why: format!("{e:#}"),
                }),
            }
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        // Two directories cannot produce one name — `Skill::load` pins the
        // name to the directory — so a duplicate is impossible rather than
        // resolved by a rule nobody would remember.
        (SkillStore { skills }, errors)
    }

    pub fn all(&self) -> &[Skill] {
        &self.skills
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// The skills a run actually carries.
    ///
    /// `enabled` empty means all of them; `disabled` is applied after, so it
    /// wins. Order is preserved, which is to say still sorted.
    pub fn select(&self, enabled: &[String], disabled: &[String]) -> Vec<Skill> {
        let disabled: BTreeSet<&str> = disabled.iter().map(String::as_str).collect();
        self.skills
            .iter()
            .filter(|s| enabled.is_empty() || enabled.iter().any(|e| e == &s.name))
            .filter(|s| !disabled.contains(s.name.as_str()))
            .cloned()
            .collect()
    }

    /// Names in `enabled`/`disabled` that match no skill on disk.
    ///
    /// Worth saying at startup for the reason a routed outbox name matching no
    /// tool is: a typo'd enable is indistinguishable from a skill the model
    /// never chose, and both look like nothing happening.
    pub fn unknown_names<'a>(&self, names: &'a [String]) -> Vec<&'a str> {
        names
            .iter()
            .map(String::as_str)
            .filter(|n| self.get(n).is_none())
            .collect()
    }
}

/// The level-1 block: what every run carries about skills it has not loaded.
///
/// `None` when there are none, so a machine with no skills sends no block at
/// all rather than a header explaining an empty list.
pub fn prompt_block(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut out = String::from(
        "## Skills\n\n\
         Procedures the user has written for you. Each is a name and when to use it; \
         the steps arrive only when you ask for them. Call the `skill` tool with the \
         name to load one *before* starting work it covers, and then follow it — it is \
         the user's own instruction, more specific than your general judgement.\n\n",
    );
    for skill in skills {
        out.push_str(&skill.summary_line());
        out.push('\n');
    }
    Some(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        PathBuf::from("/tmp/skills/x")
    }

    #[test]
    fn the_standard_spelling_parses() {
        // Byte-for-byte the shape of the two SKILL.md files already in this
        // repository, and of every published skill.
        let raw = "---\nname: handoff\ndescription: Update the handoff docs. Use at the end of a session.\n---\n\n# Closing out\n\nStep one.\n";
        let s = Skill::parse(raw, &dir()).unwrap();
        assert_eq!(s.name, "handoff");
        assert!(s.description.starts_with("Update the handoff"));
        assert_eq!(s.body, "# Closing out\n\nStep one.");
        assert!(s.triggers.is_empty());
        assert_eq!(s.tools, None);
    }

    #[test]
    fn the_optional_fields_parse_in_both_list_spellings() {
        let flow = "---\nname: a\ndescription: d\ntriggers: [one, \"two\"]\n---\nbody\n";
        assert_eq!(
            Skill::parse(flow, &dir()).unwrap().triggers,
            vec!["one", "two"]
        );
        let block = "---\nname: a\ndescription: d\ntools:\n  - fs_read\n  - fs_list\n---\nbody\n";
        assert_eq!(
            Skill::parse(block, &dir()).unwrap().tools.unwrap(),
            vec!["fs_read", "fs_list"]
        );
    }

    #[test]
    fn real_yaml_means_folded_scalars_work() {
        // Why this is a YAML parser and not a subset reader: a long
        // description is exactly what a folded scalar is for, and every other
        // harness reading this file accepts one.
        let raw = "---\nname: a\ndescription: >-\n  a long description\n  folded over two lines\n---\nbody\n";
        let s = Skill::parse(raw, &dir()).unwrap();
        assert_eq!(s.description, "a long description folded over two lines");
    }

    #[test]
    fn a_key_another_harness_understands_does_not_stop_it_loading_here() {
        // Portability is the whole argument for the format, so an unknown key
        // is ignored rather than refused.
        let raw = "---\nname: a\ndescription: d\nlicense: MIT\nallowed-tools: [Bash]\n---\nbody\n";
        assert_eq!(Skill::parse(raw, &dir()).unwrap().name, "a");
    }

    #[test]
    fn a_field_mecha_knows_and_cannot_use_is_refused() {
        // The other half of that rule: a wrong type on a known key, or a
        // missing required one, is an authoring mistake rather than a
        // portability one, and silence there loses a field nobody notices.
        for bad in [
            "---\nname: a\ndescription:\n  nested: map\n---\nbody\n",
            "---\nname: [a, b]\ndescription: d\n---\nbody\n",
            "---\nname: a\ndescription: d\ntools: fs_read\n---\nbody\n",
            "---\ndescription: d\n---\nbody\n",
            "---\nname: a\n---\nbody\n",
        ] {
            assert!(
                Skill::parse(bad, &dir()).is_err(),
                "should have refused: {bad:?}"
            );
        }
    }

    #[test]
    fn a_repeated_key_is_refused_rather_than_silently_resolved() {
        // Two plausible readings, and taking one quietly is how a skill ends
        // up doing something its author did not write.
        let raw = "---\nname: a\ndescription: first\ndescription: second\n---\nbody\n";
        assert!(Skill::parse(raw, &dir()).is_err());
    }

    #[test]
    fn frontmatter_that_never_closes_is_refused() {
        let raw = "---\nname: a\ndescription: d\n\n# body with no close\n";
        let e = Skill::parse(raw, &dir()).unwrap_err().to_string();
        assert!(e.contains("never closed"), "{e}");
    }

    #[test]
    fn a_file_with_no_frontmatter_says_so() {
        let e = Skill::parse("# just a document\n", &dir())
            .unwrap_err()
            .to_string();
        assert!(e.contains("no frontmatter"), "{e}");
    }

    #[test]
    fn the_names_the_standard_reserves_are_refused() {
        assert!(validate_name("claude-helper").is_err());
        assert!(validate_name("my-anthropic-thing").is_err());
        assert!(validate_name("Rec-Letter").is_err(), "uppercase");
        assert!(validate_name("rec letter").is_err(), "space");
        assert!(validate_name(&"a".repeat(65)).is_err(), "too long");
        assert!(validate_name("rec-letter-2").is_ok());
    }

    #[test]
    fn a_description_that_could_close_a_prompt_section_is_refused() {
        // It rides in every run's system prompt, so a stray tag is not a
        // cosmetic problem.
        assert!(validate_description("does <thing>").is_err());
        assert!(validate_description("  ").is_err());
        assert!(validate_description(&"d".repeat(1025)).is_err());
    }

    #[test]
    fn an_empty_tool_list_is_refused_rather_than_read_as_no_tools() {
        let raw = "---\nname: a\ndescription: d\ntools: []\n---\nbody\n";
        let e = Skill::parse(raw, &dir()).unwrap_err().to_string();
        assert!(e.contains("omit the key"), "{e}");
    }

    #[test]
    fn selection_is_all_by_default_and_disabled_wins() {
        let store = SkillStore {
            skills: vec![skill("a"), skill("b"), skill("c")],
        };
        let names = |v: Vec<Skill>| v.into_iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(names(store.select(&[], &[])), vec!["a", "b", "c"]);
        assert_eq!(
            names(store.select(&["a".into(), "b".into()], &[])),
            vec!["a", "b"]
        );
        assert_eq!(
            names(store.select(&["a".into(), "b".into()], &["b".into()])),
            vec!["a"],
            "disabled is applied after enabled, so it wins"
        );
    }

    #[test]
    fn a_name_nothing_on_disk_matches_is_reported() {
        let store = SkillStore {
            skills: vec![skill("a")],
        };
        assert_eq!(
            store.unknown_names(&["a".into(), "typo".into()]),
            vec!["typo"]
        );
    }

    #[test]
    fn an_empty_store_contributes_no_block_at_all() {
        assert_eq!(prompt_block(&[]), None);
    }

    #[test]
    fn the_block_lists_skills_in_the_order_it_was_given() {
        // Sorted upstream, in `load`, because this block is the front of the
        // cached prefix.
        let block = prompt_block(&[skill("alpha"), skill("beta")]).unwrap();
        let a = block.find("alpha").unwrap();
        let b = block.find("beta").unwrap();
        assert!(a < b, "{block}");
    }

    fn skill(name: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: "does a thing. Use when a thing is needed.".into(),
            triggers: Vec::new(),
            tools: None,
            body: "step one".into(),
            dir: dir(),
        }
    }
}
