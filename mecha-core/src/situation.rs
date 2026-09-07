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
//! run can be matched against at start — [`Situation::scope`] — and that
//! is the tool set, the workspace and the surface: `prepare` knows the
//! registry when it renders the rules block, the workspace it matched against
//! (`setup::prepare_tools` canonicalises it), and the surface the front-end
//! told it (`GlobalOpts::surface`, set by the front-end that owns the run
//! and never by a flag; the test override marks the session record and
//! never the match, or a smoke test and every `mecha exp` trial would
//! render a block with no surface-scoped rule in it). **The recorded key is the matched key by
//! construction**, as the tool list already was: the run record keeps the
//! workspace and surface the block was matched against
//! (`RunConfig::rules_workspace` and `rules_surface`, from `RulesCarried`),
//! and the miner, the backfill, the validator's region and the probe all
//! read those — never the session's jail, never `SessionMeta::kind`; the
//! miner and the backfill read the record covering the intervention's
//! message (`Transcript::config_covering`), since a resumed question or
//! a `/model` switch gives one session runs matched on different keys. The
//! two differ where one block serves many jails: `serve` renders once
//! against the producer root and jails each session a level below, Slack
//! renders against its configured workspace and jails each thread under
//! `~/.mecha/work/slack/`; a lesson stamped with the jail scoped its rule
//! to a workspace no match presents, dark forever with nothing warning
//! (found on review). The board's task door on `serve` is the surface
//! case of the same shape: the session is recorded as a task and the
//! block was matched as web. A key joins [`Situation::scope`] and
//! [`Situation::matches`] in the same change, pinned by
//! `scope_keys_and_matching_move_together`.
//!
//! The workspace became a key on 2026-09-07, after region widening existed
//! (§17.4), and not before: scoped to a workspace with no way to widen, a
//! rule learned in the one workspace most reflections come from would have
//! been dark everywhere else for good — the failure the narrower key is
//! meant to prevent, inverted. With widening, a lesson restated verbatim
//! from a second workspace's batch drops the key by intersection
//! (`learning::finalize_region_rules`), and a rule convicted in one
//! workspace and clean in another narrows to the one it held in. A rule
//! scoped before this key carries no workspace and rides in every
//! workspace, as it did; it is rewritable only by a batch whose region has
//! none either (`rewritable_in` is equality), so a single-workspace batch
//! shows it as context rather than narrowing it on no conviction.
//!
//! A run record from before `rules_workspace` and `rules_surface` gives the
//! miner neither key, and its reflections scope by tools alone — no key,
//! never a guess. Rows stamped before the fields carry the session's jail
//! and its kind, and were inert until each became a key: every `mecha
//! reflect` pass reconciles them against the run record before anything is
//! mined or batched (`learning::reconcile_key` decides per key,
//! `LearningStore::reconcile_keys` writes), to a key some attach of the
//! session presented or to none — never adding a key a row did not carry,
//! and leaving a row whose session cannot be read as it is. Not a flag a
//! human runs once: the nightly's `learn --auto` follows the pass, and a
//! rule scoped to a jail could never be consolidated back out.

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
    /// The workspace a match presents (see the module doc). Read through
    /// [`known_workspace`] like the two construction doors, because a
    /// front-end that records none writes the empty path, and a row read
    /// back as a set key would scope tonight's rules to a workspace no run
    /// presents (found on review).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_known_workspace"
    )]
    pub workspace: Option<PathBuf>,
}

fn de_known_workspace<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
    let raw: Option<PathBuf> = Option::deserialize(d)?;
    Ok(known_workspace(raw.as_deref()))
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
            workspace: known_workspace(workspace),
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
            workspace: known_workspace(workspace),
        }
    }

    /// The surface a run is on — what the front-end told `prepare`, and
    /// what the run record keeps as `rules_surface`. `None` is unknown,
    /// which matches no surface-scoped rule.
    pub fn on(mut self, surface: Option<SessionKind>) -> Situation {
        self.surface = surface;
        self
    }

    /// The tool the record is *about* — the last one the trace touched.
    /// `None`, batching as standing, in two cases: the last tool is a
    /// front-end tool ([`Self::FRONTEND_TOOLS`]) — a lesson from refusing
    /// `ask_user` is about asking, not about whatever tool ran before it —
    /// or the trigger is a followup, whose window is stale by construction:
    /// a followup is a later user turn carrying no tool results, so the
    /// names beside it are from an earlier assistant turn, and "you are too
    /// verbose" scoped to whatever ran last would be dark in every run
    /// without that tool (found on review). The window stays recorded on
    /// the reflection as the evidence the reflector was shown.
    pub fn focus(&self) -> Option<&str> {
        if self.trigger.as_deref() == Some("followup") {
            return None;
        }
        self.tools
            .last()
            .map(String::as_str)
            .filter(|t| !Self::FRONTEND_TOOLS.contains(t))
    }

    /// The keys a run can be matched against at start: the tool set, the
    /// workspace and the surface. Tools sorted, because a scope is a set
    /// and two batches whose regions are the same tools in another order
    /// must be the same region; and without the front-end tools, which no
    /// run registers at match time. The trigger is never a key (see its
    /// field).
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
            surface: self.surface,
            workspace: self.workspace.clone(),
        }
    }

    /// A scope with no keys: the rule applies everywhere, and rides in the
    /// prefix of every run as rules always did.
    pub fn is_standing(&self) -> bool {
        self.scope() == Situation::default()
    }

    /// The canonical name of a region: its scope's tools, sorted and joined
    /// by a comma, then ` @ ` and the workspace when the scope names one;
    /// empty for standing. What a per-region tally is keyed on, so two
    /// windows that touched the same tools in another order fold into one
    /// row, and two workspaces do not.
    pub fn key(&self) -> String {
        let scope = self.scope();
        let mut parts: Vec<String> = Vec::new();
        let tools = scope.tools.join(",");
        if !tools.is_empty() {
            parts.push(tools);
        }
        if let Some(w) = &scope.workspace {
            parts.push(format!("@ {}", w.display()));
        }
        if let Some(k) = scope.surface {
            parts.push(format!("on {}", k.as_str()));
        }
        parts.join(" ")
    }

    /// Whether a rule scoped to `self` belongs in `run`'s prefix. Every
    /// scope key `self` sets must hold in `run`; a key `self` does not set
    /// constrains nothing. Three keys: every tool the scope names is in the
    /// run's registry; the workspace the scope names, if any, is the one
    /// the run is jailed to — exactly, since both sides carry the canonical
    /// path, and a jail is not a prefix; and the surface the scope names,
    /// if any, is the one the run's front-end declared — a run that
    /// declared none matches no surface-scoped rule.
    pub fn matches(&self, run: &Situation) -> bool {
        self.tools.iter().all(|t| run.tools.contains(t))
            && self
                .workspace
                .as_ref()
                .is_none_or(|w| run.workspace.as_ref() == Some(w))
            && self.surface.is_none_or(|k| run.surface == Some(k))
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

/// The door for the workspace key — all three of them: [`Situation::recorded`],
/// [`Situation::of_run`], and the field's deserializer. An empty path is
/// *no* workspace, never a workspace named `""`. A session record whose
/// front-end recorded none carries the empty path (`SessionMeta::workspace`
/// is not optional, and the Slack connector writes `PathBuf::default()` —
/// found on review), and mapped straight through it became a set key no
/// run could ever present: a rule scoped to it was dark everywhere, the
/// roster printed the key as though it meant something, and nothing
/// warned. The read door matters as much as the two construction doors:
/// the store is append-only, and a row a past build wrote is read on
/// this build's terms (found on review, the next pass). Unknown is not a
/// match and not a key.
fn known_workspace(workspace: Option<&Path>) -> Option<PathBuf> {
    workspace
        .filter(|w| !w.as_os_str().is_empty())
        .map(Path::to_path_buf)
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
        let w = Path::new("/w");
        let with = Situation::of_run(&["fs_read".into(), "shell".into()], Some(w))
            .on(Some(SessionKind::Tui));
        let without = Situation::of_run(&["fs_read".into()], Some(w)).on(Some(SessionKind::Tui));
        assert!(scope.matches(&with));
        assert!(!scope.matches(&without));
        // Standing constrains nothing.
        assert!(Situation::default().matches(&without));
        assert!(Situation::default().is_standing());
        assert!(!scope.is_standing());
        // A workspace alone is a scope: not standing, and it loads only there.
        let here = Situation::recorded(&[], "steer", None, Some(w)).scope();
        assert!(!here.is_standing());
        assert!(here.matches(&without));
        assert!(!here.matches(&Situation::of_run(&["fs_read".into()], None)));
    }

    /// Every key a reflection records is a scope key except the trigger,
    /// and the two halves move together: if a key joins `scope`, it must
    /// join `matches` in the same change, or a rule scoped by it loads
    /// everywhere (or nowhere) while the roster prints the key as though
    /// it meant something.
    #[test]
    fn scope_keys_and_matching_move_together() {
        let full = s(&["shell"]);
        let scope = full.scope();
        assert_eq!(
            scope.trigger, None,
            "how a lesson was learned is not where it applies"
        );
        assert_eq!(scope.surface, Some(SessionKind::Tui));
        assert_eq!(scope.workspace.as_deref(), Some(Path::new("/w")));
        // The same surface in the same workspace matches; another surface
        // does not, and neither does a run that declared none.
        let same = Situation {
            tools: vec!["shell".into()],
            trigger: None,
            surface: Some(SessionKind::Tui),
            workspace: Some(PathBuf::from("/w")),
        };
        assert!(scope.matches(&same));
        assert!(full.matches(&same));
        let other_surface = Situation {
            surface: Some(SessionKind::Trigger),
            ..same.clone()
        };
        assert!(!scope.matches(&other_surface));
        assert!(!scope.matches(&Situation {
            surface: None,
            ..same.clone()
        }));
        // A scope that names no surface rides on every surface.
        let anywhere =
            Situation::recorded(&["shell".into()], "denial", None, Some(Path::new("/w")));
        assert!(anywhere.scope().matches(&other_surface));
        // Another workspace does not, and neither does a run that records
        // none: a key the scope sets must hold, and unknown is not a match.
        let elsewhere = Situation {
            workspace: Some(PathBuf::from("/elsewhere")),
            ..same.clone()
        };
        assert!(!scope.matches(&elsewhere));
        assert!(
            !scope.matches(&Situation::of_run(&["shell".into()], None).on(Some(SessionKind::Tui)))
        );
        // A jail is not a prefix: a run rooted below the scope's workspace
        // is another workspace.
        let below = Situation {
            workspace: Some(PathBuf::from("/w/sub")),
            ..same
        };
        assert!(!scope.matches(&below));
        // A scope from before the key carries no workspace and rides in
        // every workspace, as it did.
        let old: Situation = serde_json::from_str(r#"{"tools":["shell"]}"#).unwrap();
        assert!(old.matches(&elsewhere));
        assert!(old.matches(&below));
        assert!(old.matches(&Situation::of_run(&["shell".into()], None)));
    }

    /// Two members on different surfaces share none, so the region — and
    /// a widening by intersection — drops the key; the key names the
    /// surface after the workspace.
    #[test]
    fn a_region_across_surfaces_drops_the_surface_key() {
        let a = s(&["shell"]);
        let mut b = s(&["shell"]);
        b.surface = Some(SessionKind::Slack);
        let region = Situation::region([&a, &b]).scope();
        assert_eq!(region.surface, None);
        assert_eq!(region.workspace.as_deref(), Some(Path::new("/w")));
        assert_eq!(
            Situation::region([&a, &s(&["shell"])]).scope().surface,
            Some(SessionKind::Tui)
        );
        assert_eq!(s(&["shell"]).key(), "shell @ /w on tui");
        assert_eq!(
            Situation::recorded(&["shell".into()], "denial", Some(SessionKind::Web), None).key(),
            "shell on web"
        );
    }

    /// A session that recorded no workspace carries the empty path, and
    /// the empty path is not a key: a rule learned from it scopes by tools
    /// alone and loads in every workspace, rather than in none. Fails on
    /// the pass-through, which scoped it to `""`.
    #[test]
    fn an_empty_workspace_is_no_workspace() {
        let none = Situation::recorded(&["shell".into()], "denial", None, Some(Path::new("")));
        assert_eq!(none.workspace, None);
        assert_eq!(none.scope().key(), "shell");
        assert!(none
            .scope()
            .matches(&Situation::of_run(&["shell".into()], Some(Path::new("/w")))));
        assert_eq!(
            Situation::of_run(&["shell".into()], Some(Path::new(""))).workspace,
            None
        );
        assert!(
            Situation::recorded(&[], "denial", None, Some(Path::new("")))
                .scope()
                .is_standing()
        );
        // The read door too: a row written by a build that recorded the
        // empty path comes back with no workspace — not `""`.
        let on_disk: Situation =
            serde_json::from_str(r#"{"tools":["shell"],"trigger":"denial","workspace":""}"#)
                .unwrap();
        assert_eq!(on_disk.workspace, None);
        assert_eq!(on_disk.scope().key(), "shell");
        let kept: Situation =
            serde_json::from_str(r#"{"tools":["shell"],"workspace":"/w"}"#).unwrap();
        assert_eq!(kept.workspace.as_deref(), Some(Path::new("/w")));
    }

    /// Two members in different workspaces share no workspace, so the
    /// region — and a widening by intersection — drops the key; two in
    /// the same one keep it.
    #[test]
    fn a_region_across_workspaces_drops_the_workspace_key() {
        let a = s(&["shell"]);
        let mut b = s(&["shell"]);
        b.workspace = Some(PathBuf::from("/elsewhere"));
        let region = Situation::region([&a, &b]).scope();
        assert_eq!(region.tools, vec!["shell"]);
        assert_eq!(region.workspace, None);
        assert_eq!(
            Situation::region([&a, &s(&["shell", "fs_read"])])
                .scope()
                .workspace
                .as_deref(),
            Some(Path::new("/w"))
        );
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
        // A followup's window is from an earlier turn: recorded, never a focus.
        let followup = Situation::recorded(&["shell".into()], "followup", None, None);
        assert_eq!(followup.tools, vec!["shell"]);
        assert_eq!(followup.focus(), None);
        // A front-end focus in a workspace scopes to the workspace alone —
        // the standing bucket is decided by the focus (`batches_by_region`),
        // not by the scope — and with no workspace recorded it is standing.
        assert!(!s(&["ask_user"]).scope().is_standing());
        assert!(s(&["ask_user"]).scope().tools.is_empty());
        assert!(
            Situation::recorded(&["ask_user".into()], "denial", None, None)
                .scope()
                .is_standing()
        );
    }

    /// A region's key is its scope, so two windows that differ in order,
    /// trigger, surface or a front-end tool fold into one tally row, two
    /// workspaces make two rows, and standing is the empty key.
    #[test]
    fn the_key_is_the_scope_and_nothing_else() {
        assert_eq!(s(&["shell", "fs_read"]).key(), "fs_read,shell @ /w on tui");
        assert_eq!(
            s(&["fs_read", "shell", "ask_user"]).key(),
            "fs_read,shell @ /w on tui"
        );
        let nowhere = Situation::recorded(&["shell".into()], "denial", None, None);
        assert_eq!(nowhere.key(), "shell");
        assert_eq!(Situation::default().key(), "");
        assert_eq!(
            s(&["ask_user"]).key(),
            "@ /w on tui",
            "a front-end focus in a workspace is still that workspace"
        );
        assert_eq!(
            Situation::recorded(&["ask_user".into()], "denial", None, None).key(),
            ""
        );
    }

    #[test]
    fn describe_names_the_keys_and_standing_says_so() {
        assert_eq!(s(&["shell"]).describe(), "shell · denial · tui · /w");
        assert_eq!(s(&["shell"]).scope().describe(), "shell · tui · /w");
        assert_eq!(Situation::default().describe(), "everywhere");
    }
}
