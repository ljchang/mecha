//! The `skill` tool: level 2 of progressive disclosure.
//!
//! Every enabled skill's name and description already ride in the system
//! prompt. This is how the model gets the *body* — the actual procedure — and
//! it is a tool call rather than a `cat` for four reasons, all of them mecha's
//! rather than the standard's:
//!
//! - `shell` may be sandboxed, or withheld entirely. A loading mechanism that
//!   depends on it stops working in exactly the configurations that were
//!   locked down on purpose.
//! - A tool call passes the `pre_tool` gate, so a policy hook can decide which
//!   skills may load. A `cat` is invisible to hooks.
//! - It lands in the trace, so an eval case can assert on it and
//!   `sessions health` can count it. A silent context injection is the thing
//!   Datadog named as defeating every downstream defence.
//! - The model does not have to know where the filesystem keeps things.
//!
//! ## It arms no taint, and that is the point
//!
//! A skill body is user-authored — there is no install verb, no remote fetch,
//! and nothing here is ever written by a model. So it is the user's own words,
//! exactly like the system prompt, and this tool declares
//! [`Capabilities::default`] and returns [`ToolOutput::ok`] rather than
//! `from_outside`. Marking it untrusted would be a category error in the
//! direction that makes a model invent explanations for its own harness — the
//! same mistake as labelling a harness refusal as third-party content. See
//! [`crate::skill`] for the provenance argument this rests on.

use super::{Capabilities, CarriedState, Tool, ToolCtx, ToolOutput};
use crate::skill::Skill;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;

/// Loads skill bodies, and remembers which it has loaded.
pub struct SkillTool {
    /// The enabled set, sorted, fixed for the life of the agent.
    ///
    /// **Never model input**, in the way `recall`'s transcript path is never
    /// model input: the model names a skill, and the name is looked up in this
    /// list. There is no argument that reaches the filesystem, so no call can
    /// read a file the user did not put in the store.
    available: Vec<Skill>,
    /// Names loaded so far, in load order.
    loaded: Mutex<Vec<String>>,
}

impl SkillTool {
    pub fn new(available: Vec<Skill>) -> Self {
        SkillTool {
            available,
            loaded: Mutex::new(Vec::new()),
        }
    }

    /// What has been loaded, for a UI or a test.
    pub fn loaded(&self) -> Vec<String> {
        self.loaded.lock().unwrap().clone()
    }

    /// What this run actually carries — the level-1 set, after config
    /// selection and `--skill` narrowing.
    ///
    /// For a UI answering "what does this agent know how to do". It has to
    /// come from here rather than from re-reading the store beside the
    /// config, because `--skill` narrows the run without touching either:
    /// `mecha skills` shipped with exactly that bug, marking every
    /// config-selected skill as carried while the run carried one.
    pub fn available(&self) -> &[Skill] {
        &self.available
    }

    /// Forget what is loaded, because the conversation that loaded it ended.
    ///
    /// A loaded skill is the agent's state and a **conversation** is the scope
    /// it belongs to. Where one agent serves one conversation nothing needs to
    /// call this; where a front-end starts a fresh one — `/clear`, the next
    /// batch item — it has to, or a `tools:` narrowing outlives the task that
    /// asked for it and silently constrains the next one. There is no unload
    /// *within* a conversation on purpose: a procedure that has been read
    /// cannot be un-read, and the narrowing is the fail-closed direction.
    pub fn clear(&self) {
        self.loaded.lock().unwrap().clear();
    }

    fn skill(&self, name: &str) -> Option<&Skill> {
        self.available.iter().find(|s| s.name == name)
    }

    /// The body as the model receives it.
    ///
    /// Level 3 is routed back through this tool rather than pointed at the
    /// filesystem, and that is not a stylistic choice: a skill lives in
    /// `~/.mecha/skills/`, which is **outside the run's workspace**, so the
    /// path jail refuses it — correctly. Telling the model to `fs_read` a
    /// bundled file produced a call that could not succeed, found by running
    /// it. Serving the file here keeps the jail intact and gives the bundled
    /// files their own containment proof, rooted at the skill's own directory.
    fn render(skill: &Skill) -> String {
        format!(
            "# Skill: {}\n\
             If this procedure points at a file bundled with it, call `skill` \
             again with `file` set to that name — the ordinary file tools \
             cannot reach it, since a skill lives outside the workspace.\n\n{}",
            skill.name, skill.body
        )
    }

    /// Resolve a bundled file inside one skill's directory.
    ///
    /// The path jail's rule applied to a second root: canonicalize, then prove
    /// containment. `file` is the only argument on this tool that a model can
    /// point at the filesystem, so it gets the treatment every model-supplied
    /// path gets — `..` cannot climb out, and a symlink cannot either, because
    /// containment is checked after canonicalization rather than on the string.
    fn resolve_bundled(skill: &Skill, file: &str) -> Result<PathBuf, String> {
        let root = skill
            .dir
            .canonicalize()
            .map_err(|e| format!("cannot read the skill's directory: {e}"))?;
        let candidate = root.join(file);
        let resolved = candidate
            .canonicalize()
            .map_err(|_| format!("no file `{file}` bundled with skill `{}`", skill.name))?;
        if !resolved.starts_with(&root) {
            return Err(format!(
                "`{file}` resolves outside skill `{}` — a bundled file has to be \
                 inside the skill's own directory",
                skill.name
            ));
        }
        if !resolved.is_file() {
            return Err(format!("`{file}` is not a file"));
        }
        Ok(resolved)
    }
}

/// Ceiling on one bundled file.
///
/// Level 3's whole promise is "zero cost until read", which stops being true
/// if one read can swallow the window. Generous enough for a reference
/// document and small enough that a stray binary cannot end a run — and the
/// message says what was cut, because a silently truncated reference is a
/// procedure with steps missing.
const MAX_BUNDLED_BYTES: usize = 60_000;

/// Ceiling on everything carried across one compaction.
///
/// A compaction exists to make the prompt smaller, so what it reinstalls has
/// to be bounded or the mechanism fights itself. Comfortably larger than the
/// standard's own level-2 guidance (a body under ~5k tokens), so a run
/// working through two ordinary procedures never notices it.
const CARRIED_BUDGET: usize = 24_000;

/// The largest index at or below `max` that a string may be split at.
///
/// `str::floor_char_boundary` is still unstable, and slicing mid-character
/// panics — in a tool whose whole job is reading files somebody else wrote.
fn floor_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        "Load the full instructions for one of the skills listed in your system \
         prompt. Call this before starting work the skill covers, then follow what \
         it says. The skills are procedures the user wrote for you, so they are more \
         specific than your general judgement about how to do the task."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill's name, exactly as listed in the system prompt."
                },
                "file": {
                    "type": "string",
                    "description": "Optional: a file bundled with the skill, named by its                                     procedure. Omit to load the procedure itself."
                }
            },
            "required": ["name"]
        })
    }

    /// Reading a local file the user wrote, with no side effect anyone can
    /// observe. Skipping the approval gate matters more than it looks: a
    /// procedure the user authored should not need a click to be *read*, or
    /// every run that follows instructions costs an extra interruption.
    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(name) = input.get("name").and_then(Value::as_str) else {
            return Ok(ToolOutput::err("`name` is required, and must be a string"));
        };
        let name = name.trim();

        let Some(skill) = self.skill(name) else {
            let known: Vec<&str> = self.available.iter().map(|s| s.name.as_str()).collect();
            return Ok(ToolOutput::err(if known.is_empty() {
                "no skills are enabled for this run".to_string()
            } else {
                format!("no skill named `{name}`. Enabled: {}", known.join(", "))
            }));
        };

        // Level 3: a file the procedure pointed at. Deliberately does not
        // count as loading the skill — the body is what carries the
        // instructions, and a run that read a reference without the procedure
        // has not adopted it.
        if let Some(file) = input.get("file").and_then(Value::as_str) {
            return Ok(match Self::resolve_bundled(skill, file.trim()) {
                Err(why) => ToolOutput::err(why),
                Ok(path) => match std::fs::read_to_string(&path) {
                    Err(e) => ToolOutput::err(format!("cannot read `{file}`: {e}")),
                    Ok(text) if text.len() > MAX_BUNDLED_BYTES => {
                        // The check counts bytes, so the cut must too. Taking
                        // *characters* here made the ceiling a lie in both
                        // directions: a 90 KB file of three-byte characters
                        // tripped the check and then came back whole with a
                        // message claiming it had been cut.
                        let end = floor_char_boundary(&text, MAX_BUNDLED_BYTES);
                        ToolOutput::ok(format!(
                            "{}\n\n[cut: `{file}` is {} bytes, over the {MAX_BUNDLED_BYTES}-byte \
                             ceiling for one bundled file]",
                            &text[..end],
                            text.len()
                        ))
                    }
                    Ok(text) => ToolOutput::ok(text),
                },
            });
        }

        let mut loaded = self.loaded.lock().unwrap();
        if !loaded.iter().any(|n| n == name) {
            loaded.push(name.to_string());
        }
        drop(loaded);

        // Re-loading returns the body again rather than "already loaded":
        // after a compaction the model may genuinely no longer hold it, and a
        // tool that answers a request for instructions by declining to give
        // them is the shape that makes a run go in circles.
        Ok(ToolOutput::ok(Self::render(skill)))
    }

    /// Loaded skills cross a compaction verbatim.
    ///
    /// A summariser preserves what is true and drops how far you got — and for
    /// a procedure it does something worse, because a *paraphrased* procedure
    /// is a different procedure. The steps would survive as a plausible
    /// gist with the specifics gone, which is exactly the failure the user
    /// wrote the skill to prevent. `rebuild` places carried state after the
    /// summary, as the part of the rebuilt head known to be current rather
    /// than paraphrased.
    fn carried_state(&self) -> Option<CarriedState> {
        let loaded = self.loaded.lock().unwrap();
        if loaded.is_empty() {
            return None;
        }
        // Bounded, because this is re-inserted at *every* compaction and
        // nothing bounds a `SKILL.md` body — the parser caps `name` and
        // `description`, not the procedure. Two long skills carried unbounded
        // could land the rebuilt transcript back at the threshold, spending a
        // summary per turn and arming the loop guard.
        //
        // Newest first, on `collapse_repeated_failures`' reasoning: the most
        // recently loaded procedure is the one the run is most likely working
        // through. What does not fit is *named* rather than dropped silently,
        // so the model can reload it deliberately.
        let mut kept: Vec<String> = Vec::new();
        let mut dropped: Vec<&str> = Vec::new();
        let mut budget = CARRIED_BUDGET;
        for skill in loaded.iter().rev().filter_map(|n| self.skill(n)) {
            let rendered = Self::render(skill);
            if rendered.len() <= budget {
                budget -= rendered.len();
                kept.push(rendered);
            } else {
                dropped.push(skill.name.as_str());
            }
        }
        if kept.is_empty() && dropped.is_empty() {
            return None;
        }
        kept.reverse();
        dropped.reverse();

        let mut body = format!(
            "Skills loaded in this session, reproduced in full because a \
             summary of a procedure is a different procedure:\n\n{}",
            kept.join("\n\n---\n\n")
        );
        if !dropped.is_empty() {
            body.push_str(&format!(
                "\n\n[also loaded earlier, too long to carry: {}. Call `skill` again if \
                 you need one of them.]",
                dropped.join(", ")
            ));
        }
        Some(CarriedState {
            label: "skill".to_string(),
            body,
        })
    }

    /// The union of what the loaded skills declared, or `None` if none did.
    ///
    /// A skill with no `tools` key is an opinion-free skill and must not drag
    /// the surface down to whatever its neighbour declared, so the union is
    /// taken over declaring skills only — and if none declares, nothing
    /// narrows. See [`Tool::narrows_surface_to`] for the composition rule.
    /// A conversation ending is the one thing that unloads a skill: loaded
    /// skills are its state, and a `tools:` narrowing that outlived it would
    /// silently constrain the next task.
    fn forget_conversation_state(&self) {
        self.clear();
    }

    fn narrows_surface_to(&self) -> Option<Vec<String>> {
        let loaded = self.loaded.lock().unwrap();
        let mut names: Vec<String> = Vec::new();
        let mut any = false;
        for skill in loaded.iter().filter_map(|n| self.skill(n)) {
            if let Some(tools) = &skill.tools {
                any = true;
                names.extend(tools.iter().cloned());
            }
        }
        any.then_some(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn skill(name: &str, tools: Option<Vec<&str>>) -> Skill {
        Skill {
            name: name.to_string(),
            description: "d".into(),
            triggers: Vec::new(),
            tools: tools.map(|t| t.into_iter().map(String::from).collect()),
            body: format!("the {name} procedure"),
            dir: PathBuf::from("/tmp/skills").join(name),
        }
    }

    async fn load(tool: &SkillTool, name: &str) -> ToolOutput {
        tool.call(json!({ "name": name }), &ToolCtx::default())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn loading_returns_the_body_verbatim_and_names_the_directory() {
        let tool = SkillTool::new(vec![skill("audit", None)]);
        let out = load(&tool, "audit").await;
        assert!(!out.is_error);
        assert!(
            out.content.contains("the audit procedure"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("call `skill` again with `file`"),
            "level 3 has to be reachable, and only through this tool: {}",
            out.content
        );
        assert_eq!(tool.loaded(), vec!["audit"]);
    }

    #[tokio::test]
    async fn a_loaded_skill_is_never_third_party_content() {
        // The provenance decision, asserted rather than left to a comment: a
        // skill is user-authored, so loading one must not arm the interlock.
        let tool = SkillTool::new(vec![skill("audit", None)]);
        let out = load(&tool, "audit").await;
        assert!(
            !out.external,
            "a user-authored procedure is not outside input"
        );
        assert_eq!(tool.capabilities(), Capabilities::default());
    }

    #[tokio::test]
    async fn an_unknown_name_lists_what_is_enabled_rather_than_failing_blind() {
        let tool = SkillTool::new(vec![skill("audit", None), skill("brief", None)]);
        let out = load(&tool, "audi").await;
        assert!(out.is_error);
        assert!(out.content.contains("audit") && out.content.contains("brief"));
        assert!(tool.loaded().is_empty(), "a failed load is not a load");
    }

    #[tokio::test]
    async fn re_loading_hands_the_body_back_rather_than_declining() {
        // After a compaction the model may genuinely no longer hold it.
        let tool = SkillTool::new(vec![skill("audit", None)]);
        let first = load(&tool, "audit").await;
        let again = load(&tool, "audit").await;
        assert_eq!(first.content, again.content);
        assert_eq!(tool.loaded(), vec!["audit"], "and it is not counted twice");
    }

    #[tokio::test]
    async fn nothing_is_carried_across_a_compaction_until_something_is_loaded() {
        let tool = SkillTool::new(vec![skill("audit", None)]);
        assert!(tool.carried_state().is_none());
        load(&tool, "audit").await;
        let carried = tool.carried_state().unwrap();
        assert!(
            carried.body.contains("the audit procedure"),
            "{}",
            carried.body
        );
    }

    #[tokio::test]
    async fn a_bundled_file_is_served_by_the_tool_itself() {
        // Level 3. It cannot go through `fs_read`: a skill lives outside the
        // run's workspace, so the path jail refuses it — which is how this
        // was found, by a real run whose read was correctly denied.
        let dir = std::env::temp_dir().join(format!("mecha-skill-l3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("reference.md"), "the long reference").unwrap();
        let mut s = skill("bundled", None);
        s.dir = dir.clone();
        let tool = SkillTool::new(vec![s]);

        let out = tool
            .call(
                json!({"name": "bundled", "file": "reference.md"}),
                &ToolCtx::default(),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "the long reference");
        assert!(
            tool.loaded().is_empty(),
            "reading a reference is not adopting the procedure"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_bundled_path_cannot_climb_out_of_its_skill() {
        // `file` is the one argument here a model can point at the filesystem,
        // so it gets the path jail's rule against a second root.
        let dir = std::env::temp_dir().join(format!("mecha-skill-esc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = skill("escape", None);
        s.dir = dir.clone();
        let tool = SkillTool::new(vec![s]);

        for bad in ["../../../etc/passwd", "/etc/passwd"] {
            let out = tool
                .call(json!({"name": "escape", "file": bad}), &ToolCtx::default())
                .await
                .unwrap();
            assert!(out.is_error, "should have refused {bad}: {}", out.content);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_multibyte_reference_is_cut_on_a_character_boundary() {
        // The check counts bytes and the cut has to as well. Taking characters
        // meant a 3-byte-per-char file tripped the ceiling and then came back
        // whole under a message claiming it had been cut.
        let dir = std::env::temp_dir().join(format!("mecha-skill-utf8-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = "é".repeat(MAX_BUNDLED_BYTES); // 2 bytes each, so twice the ceiling
        std::fs::write(dir.join("ref.md"), &big).unwrap();
        let mut s = skill("utf8", None);
        s.dir = dir.clone();
        let tool = SkillTool::new(vec![s]);

        let out = tool
            .call(
                json!({"name": "utf8", "file": "ref.md"}),
                &ToolCtx::default(),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("[cut:"),
            "it really was over the ceiling"
        );
        // The claim and the deed agree: what came back is genuinely shorter.
        assert!(
            out.content.len() < big.len(),
            "content {} vs original {}",
            out.content.len(),
            big.len()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_carried_block_is_bounded_and_names_what_would_not_fit() {
        // Re-inserted at every compaction, and nothing bounds a SKILL.md body.
        // Unbounded, two long procedures could land the rebuilt transcript
        // back at the threshold and spend a summary per turn.
        // Two thirds of the budget each: either fits alone, both cannot.
        let long = "x".repeat(CARRIED_BUDGET * 2 / 3);
        let mut a = skill("older", None);
        a.body = long.clone();
        let mut b = skill("newer", None);
        b.body = long;
        let tool = SkillTool::new(vec![a, b]);
        load(&tool, "older").await;
        load(&tool, "newer").await;

        let carried = tool.carried_state().unwrap();
        assert!(
            carried.body.len() < 2 * CARRIED_BUDGET,
            "bounded: {}",
            carried.body.len()
        );
        // Newest kept, oldest named rather than dropped in silence, so the
        // model can reload it deliberately.
        assert!(carried.body.contains("# Skill: newer"), "newest survives");
        assert!(
            carried.body.contains("too long to carry: older"),
            "and the drop is named: {}",
            carried.body
        );
    }

    #[tokio::test]
    async fn a_procedure_too_long_to_carry_is_named_rather_than_truncated() {
        // The one case where nothing is carried. Cutting a procedure in half
        // is the failure this whole mechanism exists to avoid, so an
        // oversized one is named and left to be reloaded on purpose.
        let mut huge = skill("huge", None);
        huge.body = "x".repeat(CARRIED_BUDGET * 2);
        let tool = SkillTool::new(vec![huge]);
        load(&tool, "huge").await;

        let carried = tool.carried_state().unwrap();
        assert!(
            carried.body.contains("too long to carry: huge"),
            "{}",
            carried.body
        );
        assert!(
            !carried.body.contains(&"x".repeat(100)),
            "no half a procedure"
        );
    }

    #[tokio::test]
    async fn a_conversation_ending_unloads_everything() {
        // One agent can outlive a conversation — a batch item, a `/clear` —
        // and a narrowing that survived would constrain the next task.
        let tool = SkillTool::new(vec![skill("audit", Some(vec!["fs_read"]))]);
        load(&tool, "audit").await;
        assert!(tool.narrows_surface_to().is_some());
        assert!(tool.carried_state().is_some());

        tool.forget_conversation_state();
        assert!(tool.loaded().is_empty());
        assert_eq!(
            tool.narrows_surface_to(),
            None,
            "the surface has to come back, or the next task starts constrained"
        );
        assert!(tool.carried_state().is_none());
    }

    #[tokio::test]
    async fn a_skill_that_declares_no_tools_narrows_nothing() {
        let tool = SkillTool::new(vec![skill("audit", None)]);
        load(&tool, "audit").await;
        assert_eq!(tool.narrows_surface_to(), None);
    }

    #[tokio::test]
    async fn declared_tools_narrow_and_two_skills_union() {
        let tool = SkillTool::new(vec![
            skill("audit", Some(vec!["fs_read"])),
            skill("brief", Some(vec!["mail_send"])),
        ]);
        load(&tool, "audit").await;
        assert_eq!(tool.narrows_surface_to().unwrap(), vec!["fs_read"]);

        // Union, not intersection: each skill names what its own procedure
        // needs, and intersecting would strand a run that loaded both.
        load(&tool, "brief").await;
        let both = tool.narrows_surface_to().unwrap();
        assert!(both.contains(&"fs_read".to_string()));
        assert!(both.contains(&"mail_send".to_string()));
    }

    #[tokio::test]
    async fn an_opinion_free_skill_does_not_widen_a_restriction_its_neighbour_set() {
        // The asymmetry that matters: loading a skill with no `tools` key
        // alongside one that has it must not lift the restriction.
        let tool = SkillTool::new(vec![
            skill("audit", Some(vec!["fs_read"])),
            skill("plain", None),
        ]);
        load(&tool, "audit").await;
        load(&tool, "plain").await;
        assert_eq!(tool.narrows_surface_to().unwrap(), vec!["fs_read"]);
    }
}
