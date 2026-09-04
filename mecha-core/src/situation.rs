//! Where a record was made, from the closed sets the harness already holds.
//!
//! `GoalRef` answers *what a record serves*; nothing answered *where it
//! happened*, and that absence is why a lesson learned about `shell` being
//! refused was stored as a universal behaviour rule and loaded into every
//! prompt (`docs/GOAL-SYSTEM-DESIGN.md` §17.3). A [`Situation`] is the
//! closed-set description of that *where*: cited by a reflection at mining
//! time, by a rule as the region it applies in, and by a run at start as the
//! region it is in.
//!
//! **Every key is a name from a set the harness owns.** Tool names come from
//! the registry, the trigger from [`crate::learning::Trigger`], the surface
//! from [`SessionKind`], the workspace from the session record. Never a tool
//! argument, never prose: a model authors arguments and prose, and a key a
//! model can author is a key an injection can set. That is also why the
//! reflector's `error_type` is *not* a key here even though it sits beside
//! the situation on the reflection — it is the model's label, and the design
//! lists it only as something the reflection already carries.
//!
//! ## Recorded keys and scope keys
//!
//! A reflection records every key it can. A *rule* is scoped by the subset a
//! run can be matched against at start — [`Situation::scope`] — and today
//! that is the tool set alone: `prepare` knows the registry when it renders
//! the rules block, and knows neither the surface (the front-end names it
//! when it opens the session, after `prepare` returns) nor anything a
//! workspace should widen to. Surface and workspace are recorded on the
//! reflection so the consolidation step that widens a region over its
//! sub-regions (§17.4) has them, and become scope keys when the loader can
//! match them — one edit in [`Situation::scope`] and one in
//! [`Situation::matches`], pinned together by
//! `scope_keys_and_matching_move_together`. Scoping a rule to a workspace
//! *before* widening exists would pin nearly every rule to the one workspace
//! most reflections come from and make it dark everywhere else, which is
//! the failure the narrower key would be meant to prevent, inverted.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::session::SessionKind;

/// The closed-set description of where a record was made. See the module
/// doc for what may be a key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Situation {
    /// Registry-owned tool names in the order the trace touched them,
    /// deduplicated. The **last is the focus**: for a denial it is the tool
    /// refused, for a steer the tool the model was mid-way through.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// [`crate::learning::Trigger::as_str`] of the intervention this was
    /// recorded at. How the lesson was *learned*, not where it applies — a
    /// rule learned from a denial applies whenever its tool is in play — so
    /// it is never a scope key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// The surface the session ran on. Lenient on read like
    /// `SessionMeta::kind`: a kind this build cannot name costs the field,
    /// never the record.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::session::de_lenient_kind"
    )]
    pub surface: Option<SessionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
}

impl Situation {
    /// Tools a front-end inserts into the registry *after* `setup::build`
    /// has rendered the rules block — so no run registers them at the
    /// moment a scope is matched, and a rule scoped to one could never
    /// load while the roster printed it as loading. [`Situation::scope`]
    /// drops them, [`Situation::focus`] does not batch on them, and
    /// `LearningStore::unloadable_rules` warns at startup about a file that
    /// names one anyway. The list is the closed set of such tools; the CLI
    /// pins it against what its front-ends actually insert.
    pub const FRONTEND_TOOLS: &[&str] = &["ask_user", "recall", "show_file"];

    /// The situation an intervention was recorded in.
    pub fn recorded(
        tools: &[String],
        trigger: &str,
        surface: Option<SessionKind>,
        workspace: Option<&Path>,
    ) -> Situation {
        let mut deduped: Vec<String> = Vec::new();
        for t in tools {
            if !deduped.contains(t) {
                deduped.push(t.clone());
            }
        }
        Situation {
            tools: deduped,
            trigger: Some(trigger.to_string()),
            surface,
            workspace: workspace.map(Path::to_path_buf),
        }
    }

    /// The situation a run is in at start: the registry it carries and the
    /// workspace it is jailed to. What a rule's scope is matched against.
    ///
    /// The registry at render time is the one `setup::build` has after
    /// builtins, MCP servers and subagents joined it. A front-end inserts
    /// its own tools afterwards (`ask_user`, recall, the TUI's), so those
    /// names are in `RunConfig::tools` and not in the situation the block
    /// was matched against — a rule scoped to one of them would never load.
    /// None of them is a tool a reflection's window names today, and the
    /// gap is recorded here rather than closed, since closing it means
    /// telling `prepare` what the front-end will add. The subtractive
    /// mirror exists too: a delegated task run withholds `kg_task_update`
    /// after the block is rendered, so a rule scoped to it rides in a run
    /// that no longer registers it — over-inclusive, costing prefix bytes
    /// and a rule the model cannot act on, never a rule that cannot load.
    pub fn of_run(tools: &[String], workspace: Option<&Path>) -> Situation {
        Situation {
            tools: tools.to_vec(),
            trigger: None,
            surface: None,
            workspace: workspace.map(Path::to_path_buf),
        }
    }

    /// The tool the record is *about* — the last one the trace touched.
    /// `None` when that is a front-end tool ([`Self::FRONTEND_TOOLS`]): a
    /// lesson from refusing `ask_user` is about asking, not about whatever
    /// tool ran before it, and batches as standing.
    pub fn focus(&self) -> Option<&str> {
        self.tools
            .last()
            .map(String::as_str)
            .filter(|t| !Self::FRONTEND_TOOLS.contains(t))
    }

    /// The keys a run can be matched against at start. See the module doc
    /// for why this is the tool set alone today. Sorted, because a scope is
    /// a set and two batches whose regions are the same tools in another
    /// order must be the same region; and without the front-end tools,
    /// which no run registers at match time.
    pub fn scope(&self) -> Situation {
        let mut tools: Vec<String> = self
            .tools
            .iter()
            .filter(|t| !Self::FRONTEND_TOOLS.contains(&t.as_str()))
            .cloned()
            .collect();
        tools.sort();
        tools.dedup();
        Situation {
            tools,
            trigger: None,
            surface: None,
            workspace: None,
        }
    }

    /// A scope with no keys: the rule applies everywhere, and rides in the
    /// prefix of every run as rules always did.
    pub fn is_standing(&self) -> bool {
        self.scope() == Situation::default()
    }

    /// Whether a rule scoped to `self` belongs in `run`'s prefix. Every
    /// scope key `self` sets must hold in `run`; a key `self` does not set
    /// constrains nothing. The one key today: every tool the scope names is
    /// in the run's registry.
    pub fn matches(&self, run: &Situation) -> bool {
        self.tools.iter().all(|t| run.tools.contains(t))
    }

    /// The keys every member shares — the region a batch of reflections was
    /// recorded in. Tools: the intersection, in the first member's order.
    /// Every other key: kept only when every member sets it the same way.
    /// Over no members, standing.
    pub fn region<'a>(members: impl IntoIterator<Item = &'a Situation>) -> Situation {
        let mut members = members.into_iter();
        let Some(first) = members.next() else {
            return Situation::default();
        };
        let mut out = first.clone();
        for m in members {
            out.tools.retain(|t| m.tools.contains(t));
            if out.trigger != m.trigger {
                out.trigger = None;
            }
            if out.surface != m.surface {
                out.surface = None;
            }
            if out.workspace != m.workspace {
                out.workspace = None;
            }
        }
        out
    }

    /// One line for a roster or a prompt: `shell · denial · tui`, or
    /// `everywhere` for a standing situation with nothing else recorded.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.tools.is_empty() {
            parts.push(self.tools.join(", "));
        }
        if let Some(t) = &self.trigger {
            parts.push(t.clone());
        }
        if let Some(k) = self.surface {
            parts.push(k.as_str().to_string());
        }
        if let Some(w) = &self.workspace {
            parts.push(w.display().to_string());
        }
        if parts.is_empty() {
            "everywhere".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(tools: &[&str]) -> Situation {
        Situation::recorded(
            &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            "denial",
            Some(SessionKind::Tui),
            Some(Path::new("/w")),
        )
    }

    #[test]
    fn the_focus_is_the_last_tool_touched_and_names_are_deduplicated() {
        let sit = Situation::recorded(
            &["fs_read".into(), "shell".into(), "fs_read".into()],
            "steer",
            None,
            None,
        );
        assert_eq!(sit.tools, vec!["fs_read", "shell"]);
        assert_eq!(sit.focus(), Some("shell"));
        assert_eq!(Situation::default().focus(), None);
    }

    #[test]
    fn a_region_keeps_only_what_every_member_shares() {
        let a = s(&["fs_read", "shell"]);
        let mut b = s(&["shell", "http_fetch"]);
        b.surface = Some(SessionKind::Slack);
        let region = Situation::region([&a, &b]);
        assert_eq!(region.tools, vec!["shell"]);
        assert_eq!(region.trigger.as_deref(), Some("denial"));
        assert_eq!(region.surface, None);
        assert_eq!(region.workspace.as_deref(), Some(Path::new("/w")));
        assert_eq!(Situation::region([]), Situation::default());
    }

    #[test]
    fn a_scope_matches_a_run_that_carries_every_tool_it_names() {
        let scope = s(&["shell"]).scope();
        let with = Situation::of_run(&["fs_read".into(), "shell".into()], None);
        let without = Situation::of_run(&["fs_read".into()], None);
        assert!(scope.matches(&with));
        assert!(!scope.matches(&without));
        // Standing constrains nothing.
        assert!(Situation::default().matches(&without));
        assert!(Situation::default().is_standing());
        assert!(!scope.is_standing());
    }

    /// The module doc promises surface and workspace are recorded but not
    /// yet matched. If a key joins `scope`, it must join `matches` in the
    /// same change, or a rule scoped by it loads everywhere (or nowhere)
    /// while the roster prints the key as though it meant something.
    #[test]
    fn scope_keys_and_matching_move_together() {
        let full = s(&["shell"]);
        let scope = full.scope();
        assert_eq!(scope.trigger, None);
        assert_eq!(scope.surface, None);
        assert_eq!(scope.workspace, None);
        // A run on another surface in another workspace still matches,
        // because neither is a scope key yet.
        let elsewhere = Situation {
            tools: vec!["shell".into()],
            trigger: None,
            surface: Some(SessionKind::Trigger),
            workspace: Some(PathBuf::from("/elsewhere")),
        };
        assert!(scope.matches(&elsewhere));
        assert!(full.matches(&elsewhere));
    }

    #[test]
    fn a_record_from_before_the_field_and_an_unknown_surface_both_load() {
        let old: Situation = serde_json::from_str("{}").unwrap();
        assert_eq!(old, Situation::default());
        let newer: Situation =
            serde_json::from_str(r#"{"tools":["shell"],"surface":"hologram"}"#).unwrap();
        assert_eq!(newer.tools, vec!["shell"]);
        assert_eq!(newer.surface, None);
    }

    /// A scope is a set: order does not make two regions, and a tool no
    /// run registers at match time is not a key.
    #[test]
    fn a_scope_is_sorted_and_names_no_front_end_tool() {
        let a = s(&["shell", "fs_read"]).scope();
        let b = s(&["fs_read", "shell"]).scope();
        assert_eq!(a, b);
        assert_eq!(a.tools, vec!["fs_read", "shell"]);
        let asked = s(&["shell", "ask_user"]);
        assert_eq!(asked.scope().tools, vec!["shell"]);
        assert_eq!(asked.focus(), None, "a front-end focus batches as standing");
        assert_eq!(s(&["shell"]).focus(), Some("shell"));
        assert!(s(&["ask_user"]).scope().is_standing());
    }

    #[test]
    fn describe_names_the_keys_and_standing_says_so() {
        assert_eq!(s(&["shell"]).describe(), "shell · denial · tui · /w");
        assert_eq!(Situation::default().describe(), "everywhere");
    }
}
