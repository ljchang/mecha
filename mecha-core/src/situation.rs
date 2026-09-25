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
//! from [`SessionKind`], the workspace from the session record, the goal
//! from the store the front-end read it out of (a board task, a trigger
//! file, the owner's `run --goal`) — never a goal a plan or a model named. Never a tool
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
//! is the tool set, the workspace, the surface and the goal: `prepare` knows the
//! registry when it renders the rules block, the workspace it matched against
//! (`setup::prepare_tools` canonicalises it), the surface the front-end
//! told it (`GlobalOpts::surface`, set by the front-end that owns the run
//! and never by a flag; the test override marks the session record and
//! never the match, or a smoke test and every `mecha exp` trial would
//! render a block with no surface-scoped rule in it; and every front-end
//! that declares a surface appends the run record that keeps it, so a
//! lesson mined there can be scoped to it), and the goal the front-end
//! handed it (`GlobalOpts::goal`; the module's last section). A stored
//! scope naming a surface or a goal this build cannot read matches nothing
//! rather than everything (parked verbatim as `surface_unread`, or as
//! [`GoalKey::Unread`]) — a lenient read that dropped either would widen,
//! and a downgrade must not destroy it. **The recorded key is the matched key by
//! construction**, as the tool list already was: the run record keeps the
//! workspace, surface and goal the block was matched against
//! (`RunConfig::rules_workspace`, `rules_surface` and `rules_goal`, from
//! `RulesCarried`), and the miner, the backfill, the validator's region and
//! the probe all read those — never the session's jail, never
//! `SessionMeta::kind`, never the conversation's goal anchor; the
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
//!
//! ## The goal key
//!
//! The goal joined on 2026-09-25 (`APPRAISAL-WIRING-DESIGN.md` M1, built as
//! 2c-1; `GOAL-SYSTEM-DESIGN.md` §17.3 calls it the richest key), once the
//! structural anchor (S1, phase 1a) made it non-empty. The key is the whole
//! reference, kind and id together — `trigger:morning`, `task:<uid>` —
//! because what the design keys on is *the same goal* ("last time on this
//! task", I2), and the kind alone would say nothing the surface does not:
//! a trigger run is already on the `trigger` surface, a delegated task on
//! `task`. It is the goal the **front-end handed `prepare`**, from a store
//! the owner wrote: `tasks work` its task, a trigger run its trigger, `run
//! --goal` the owner's pointer, a question continuation the asking run's
//! recorded one. A front-end that renders one block for many runs — `serve`,
//! the front door — and one whose runs have no structural goal — chat, the
//! TUI, voice, mail, Slack — declares none, because no one goal is the
//! block's. Never the conversation's anchor: a hand-over resumed on an older
//! anchor, or an anchor the owner confirmed mid-run, was not what the block
//! was matched on.
//!
//! **An absent goal never widens a scope** (APPRAISAL-RESEARCH §8.4). A rule
//! scoped to a goal loads only in a run that presents that goal — a run that
//! presents none, or one this build cannot name, matches it not — and a
//! stored goal this build cannot name matches nothing ([`GoalKey::Unread`])
//! rather than being read as none, which would be every goal; the run
//! record keeps an unnameable goal the same way, so the miner stamps the
//! parked key rather than none. The other direction is the acceptance
//! line: a reflection mined with no goal scopes with none, and its rule
//! loads under every goal exactly as it did before the key. Region and
//! widening need no edit: two members naming different goals — or one
//! naming none — share no goal, so the region drops the key, and a verbatim
//! restatement from another goal's batch widens by the same intersection.
//! Nothing to reconcile: no row carried a goal before the field, and every
//! door stamps it from the run record.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::goal::GoalRef;
use crate::session::SessionKind;

/// The goal key as stored: a reference this build can name, or the string
/// it cannot, kept verbatim. One type for both sides of a match — the
/// scope's key and the run's — and for the run record that keeps what was
/// matched (`RunConfig::rules_goal`), so a newer build's goal kind survives
/// an older build's read and write on every door and is never read as *no
/// goal*, which on a scope is every goal.
///
/// Serialised as the `kind:id` string [`GoalRef`] already writes, so a
/// stored key has one spelling whichever variant produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalKey {
    /// A goal this build names.
    Named(GoalRef),
    /// A stored goal this build cannot name — a kind a newer binary wrote,
    /// or a hand edit. Matches no run and is matched by no scope.
    Unread(String),
}

impl GoalKey {
    /// The reference, when this build can name it.
    pub fn named(&self) -> Option<&GoalRef> {
        match self {
            GoalKey::Named(g) => Some(g),
            GoalKey::Unread(_) => None,
        }
    }
}

impl From<GoalRef> for GoalKey {
    fn from(g: GoalRef) -> GoalKey {
        GoalKey::Named(g)
    }
}

impl std::fmt::Display for GoalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalKey::Named(g) => write!(f, "{g}"),
            GoalKey::Unread(raw) => f.write_str(raw),
        }
    }
}

impl Serialize for GoalKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

/// Lenient without widening: a string that parses is named; anything else
/// — an unknown kind, a malformed id, a value that is not a string — is
/// parked verbatim rather than read as no goal. Reached through `Option`,
/// so `null` and an absent field are none.
impl<'de> Deserialize<'de> for GoalKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<GoalKey, D::Error> {
        Ok(match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) => match GoalRef::parse_lenient(&s) {
                Some(g) => GoalKey::Named(g),
                None => GoalKey::Unread(s),
            },
            other => GoalKey::Unread(other.to_string()),
        })
    }
}

/// The closed-set description of where a record was made. See the module
/// doc for what may be a key.
///
/// Read and written through [`SituationWire`]: the load degrades, the
/// store keeps the evidence. A surface string this build cannot name lands
/// in [`Self::surface_unread`] and is written back as it came, so a
/// downgrade parks the key rather than destroying it (found on review —
/// a persisted `unknown` was never recovered by the build that could name
/// it), and an empty workspace path reads as none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SituationWire", into = "SituationWire")]
pub struct Situation {
    /// Registry-owned tool names in the order the trace touched them,
    /// deduplicated. The **last is the focus**: for a denial it is the tool
    /// refused, for a steer the tool the model was mid-way through.
    pub tools: Vec<String>,
    /// [`crate::learning::Trigger::as_str`] of the intervention this was
    /// recorded at. How the lesson was *learned*, not where it applies — a
    /// rule learned from a denial applies whenever its tool is in play — so
    /// it is never a scope key.
    pub trigger: Option<String>,
    /// The surface the block was matched against, when this build can name
    /// it. `None` here means "names no surface", and on a scope that is
    /// every surface — so a surface this build cannot name is never read
    /// as `None`: it is parked in [`Self::surface_unread`] instead, where
    /// it matches nothing (found on review: the lenient read was the one
    /// key whose malformed value widened a rule's reach). The session
    /// record keeps its lenient read, where `None` means unknown.
    pub surface: Option<SessionKind>,
    /// A stored surface this build could not name — a hand edit, or a kind
    /// a newer binary wrote — kept verbatim so it round-trips. A key that
    /// matches no run ([`Situation::matches`]), kept by [`Situation::scope`],
    /// printed by the roster, reported at startup by
    /// `LearningStore::unloadable_rules`, and replaced by the reflect
    /// pass's reconcile with a key the run record confirms. Never set by
    /// the two construction doors.
    pub surface_unread: Option<String>,
    /// The workspace a match presents (see the module doc). Read through
    /// [`known_workspace`] like the two construction doors, because a
    /// front-end that records no workspace writes the empty path, and a
    /// row read back as a set key would scope tonight's rules to a
    /// workspace no run presents (found on review).
    pub workspace: Option<PathBuf>,
    /// The goal the block was matched against (the module doc's *goal
    /// key*): the reference the front-end handed `prepare`, recorded as
    /// `RunConfig::rules_goal`. `None` names no goal, which on a scope is
    /// every goal; a stored goal this build cannot name is
    /// [`GoalKey::Unread`] and matches nothing.
    pub goal: Option<GoalKey>,
}

/// The stored shape of a [`Situation`]: `surface` is whatever the file
/// holds, so a value this build cannot name is kept rather than dropped.
#[derive(Serialize, Deserialize)]
struct SituationWire {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    surface: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    goal: Option<GoalKey>,
}

impl From<SituationWire> for Situation {
    fn from(w: SituationWire) -> Situation {
        let (surface, surface_unread) = match w.surface {
            None | Some(serde_json::Value::Null) => (None, None),
            Some(serde_json::Value::String(s)) => match SessionKind::parse_lenient(&s) {
                Some(k) => (Some(k), None),
                None => (None, Some(s)),
            },
            Some(other) => (None, Some(other.to_string())),
        };
        Situation {
            tools: w.tools,
            trigger: w.trigger,
            surface,
            surface_unread,
            workspace: known_workspace(w.workspace.as_deref()),
            goal: w.goal,
        }
    }
}

impl From<Situation> for SituationWire {
    fn from(s: Situation) -> SituationWire {
        SituationWire {
            tools: s.tools,
            trigger: s.trigger,
            surface: match (s.surface, s.surface_unread) {
                (Some(k), _) => Some(serde_json::Value::String(k.as_str().to_string())),
                (None, Some(raw)) => Some(serde_json::Value::String(raw)),
                (None, None) => None,
            },
            workspace: s.workspace,
            goal: s.goal,
        }
    }
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

    /// The surface values that are corpus marks, not places: a process
    /// puts them on the session record through `MECHA_SESSION_KIND`
    /// (`Session::test_override`), and no front-end ever declares one to
    /// `prepare`, so no run can present them at match time. The surface
    /// key's mirror of [`Self::FRONTEND_TOOLS`]: [`Situation::scope`] drops
    /// them, and `LearningStore::unloadable_rules` warns at startup about
    /// a stored scope that names one anyway — a reflection stamped `test`
    /// by the old miner, from a transcript since deleted so the reconcile
    /// left it, would otherwise have scoped a rule to a surface nothing
    /// matches, dark with nothing warning (found on review).
    pub const MARK_KINDS: &[SessionKind] = &[SessionKind::Test, SessionKind::Experiment];

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
            surface_unread: None,
            workspace: known_workspace(workspace),
            goal: None,
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
            surface_unread: None,
            workspace: known_workspace(workspace),
            goal: None,
        }
    }

    /// The surface a run is on — what the front-end told `prepare`, and
    /// what the run record keeps as `rules_surface`. `None` is unknown,
    /// which matches no surface-scoped rule.
    pub fn on(mut self, surface: Option<SessionKind>) -> Situation {
        self.surface = surface;
        self
    }

    /// The goal a situation is toward — on a run, what the front-end handed
    /// `prepare` (`GlobalOpts::goal`) or what the run record keeps as
    /// `rules_goal`; on a recorded situation, the covering run record's
    /// `rules_goal`. `None` is no goal, which matches no goal-scoped rule.
    pub fn toward(mut self, goal: Option<GoalKey>) -> Situation {
        self.goal = goal;
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
    /// workspace, the surface and the goal. Tools sorted, because a scope is a set
    /// and two batches whose regions are the same tools in another order
    /// must be the same region; without the front-end tools, which no run
    /// registers at match time; and without a surface that is a corpus
    /// mark ([`Self::MARK_KINDS`]), which no run presents. The trigger is
    /// never a key (see its field).
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
            surface: self.surface.filter(|k| !Self::MARK_KINDS.contains(k)),
            surface_unread: self.surface_unread.clone(),
            workspace: self.workspace.clone(),
            goal: self.goal.clone(),
        }
    }

    /// A scope with no keys: the rule applies everywhere, and rides in the
    /// prefix of every run as rules always did.
    pub fn is_standing(&self) -> bool {
        self.scope() == Situation::default()
    }

    /// The canonical name of a region: its scope's tools, sorted and joined
    /// by a comma, then ` @ ` and the workspace, ` on ` and the surface, and
    /// ` for ` and the goal when the scope names them (an unread surface or
    /// goal verbatim); empty for standing. What a per-region tally is keyed
    /// on, so two windows that touched the same tools in another order fold
    /// into one row, and two workspaces, two surfaces or two goals do not.
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
        if let Some(raw) = &scope.surface_unread {
            parts.push(format!("on {raw}"));
        }
        if let Some(g) = &scope.goal {
            parts.push(format!("for {g}"));
        }
        parts.join(" ")
    }

    /// Whether a rule scoped to `self` belongs in `run`'s prefix. Every
    /// scope key `self` sets must hold in `run`; a key `self` does not set
    /// constrains nothing. Four keys: every tool the scope names is in the
    /// run's registry; the workspace the scope names, if any, is the one
    /// the run is jailed to — exactly, since both sides carry the canonical
    /// path, and a jail is not a prefix; the surface the scope names,
    /// if any, is the one the run's front-end declared — a run that
    /// declared none matches no surface-scoped rule, and a scope naming a
    /// surface this build cannot read ([`Self::surface_unread`]) matches
    /// no run at all; and the goal the scope names, if any, is the one the
    /// run's front-end handed `prepare` — exactly, kind and id; a run that
    /// presents none, or one this build cannot name, matches no goal-scoped
    /// rule, and a scope whose goal this build cannot name
    /// ([`GoalKey::Unread`]) matches no run at all.
    pub fn matches(&self, run: &Situation) -> bool {
        self.tools.iter().all(|t| run.tools.contains(t))
            && self
                .workspace
                .as_ref()
                .is_none_or(|w| run.workspace.as_ref() == Some(w))
            && self.surface.is_none_or(|k| run.surface == Some(k))
            && self.surface_unread.is_none()
            && match &self.goal {
                None => true,
                Some(GoalKey::Named(g)) => run.goal.as_ref().and_then(GoalKey::named) == Some(g),
                Some(GoalKey::Unread(_)) => false,
            }
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
            if out.surface_unread != m.surface_unread {
                out.surface_unread = None;
            }
            if out.workspace != m.workspace {
                out.workspace = None;
            }
            if out.goal != m.goal {
                out.goal = None;
            }
        }
        out
    }

    /// One line for a roster or a prompt: `shell · denial · tui`, with the goal
    /// (`trigger:morning`) after the workspace when one is recorded, or
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
        if let Some(raw) = &self.surface_unread {
            parts.push(format!("{raw} (a surface this build cannot name)"));
        }
        if let Some(w) = &self.workspace {
            parts.push(w.display().to_string());
        }
        match &self.goal {
            Some(GoalKey::Named(g)) => parts.push(g.to_string()),
            Some(GoalKey::Unread(raw)) => {
                parts.push(format!("{raw} (a goal this build cannot name)"))
            }
            None => {}
        }
        if parts.is_empty() {
            "everywhere".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

// ---------------------------------------------------------- closed goals

/// Whether the goal a key names is still one a run can be handed — the
/// readout behind R34 (`APPRAISAL-WIRING-DESIGN.md` §6). A rule keeps its
/// `task:<uid>` scope when the task closes and widens only on evidence
/// (§17.4); what the ruling adds is that such a rule is **seen** to be
/// dark rather than silently riding nowhere. `LOADS NOWHERE` alone cannot
/// see it: the run that mined the rule always presented its goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalLiveness {
    /// The store says the goal is live: a task with an open board status,
    /// an enabled trigger.
    Open,
    /// The store says no run will be handed this goal again; the words say
    /// why (`task:t-9 is done`, `trigger:morning is disabled`).
    Closed(String),
    /// The store could not say — the board unread or truncated, a status
    /// this build does not know, a trigger file that does not load. Its own
    /// finding, never "not dark".
    Unknown(String),
    /// A kind this readout does not assess (a charter line, a project, a
    /// setpoint, a request) — no run is handed one as a matched goal today.
    NotAssessed,
}

/// The board's task statuses by id, off a `kg_task_list` answer read with
/// `include_closed` — what [`GoalKey::liveness`] reads a `task:` goal
/// against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardStatuses {
    by_id: std::collections::BTreeMap<String, String>,
    truncated: bool,
}

impl BoardStatuses {
    /// Read an answer: every row with an id and a status. An answer with no
    /// `items` list is not a board, and says so.
    pub fn of(answer: &serde_json::Value) -> Result<BoardStatuses, String> {
        let items = answer["items"]
            .as_array()
            .ok_or_else(|| "the answer carries no `items` list".to_string())?;
        let mut by_id = std::collections::BTreeMap::new();
        for row in items {
            if let (Some(id), Some(status)) = (row["id"].as_str(), row["status"].as_str()) {
                by_id.insert(id.to_string(), status.to_string());
            }
        }
        Ok(BoardStatuses {
            by_id,
            truncated: answer["truncated"].as_bool() == Some(true),
        })
    }
}

/// What the trigger store says about one trigger by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerState {
    Enabled,
    Disabled,
    /// No file by that name: removed, so it never fires again.
    Missing,
    /// A file that does not load, or a store that cannot be read.
    Unreadable(String),
}

impl GoalKey {
    /// Whether [`Self::liveness`] needs the board: a named `task:` goal.
    /// What gates the board read, which costs an MCP connection.
    pub fn needs_board(&self) -> bool {
        matches!(self.named(), Some(GoalRef::Task(_)))
    }

    /// Whether the goal this key names is still live, read against the
    /// stores that own it. A task is closed when its board status is
    /// closed (`closure::is_closed_status`); a task the board does not
    /// carry is unknown, since an answer can be short without saying so. A
    /// trigger is closed when its file is gone or it is disabled. Anything
    /// a store could not answer is [`GoalLiveness::Unknown`], never open.
    pub fn liveness(
        &self,
        board: &Result<BoardStatuses, String>,
        trigger: &dyn Fn(&str) -> TriggerState,
    ) -> GoalLiveness {
        let Some(goal) = self.named() else {
            return GoalLiveness::NotAssessed;
        };
        match goal {
            GoalRef::Task(id) => {
                let board = match board {
                    Ok(b) => b,
                    Err(why) => {
                        return GoalLiveness::Unknown(format!("the board could not be read: {why}"))
                    }
                };
                match board.by_id.get(id) {
                    Some(status) if crate::closure::is_closed_status(status) => {
                        GoalLiveness::Closed(format!("{goal} is {status}"))
                    }
                    Some(status) if crate::closure::is_known_status(status) => GoalLiveness::Open,
                    Some(status) => GoalLiveness::Unknown(format!(
                        "the board gives {goal} the status `{status}`, which this build does not know"
                    )),
                    // Absent is not closed: an answer can be short in a
                    // way its envelope does not say (a cap without
                    // `truncated`), the lesson `tasks::project_closed_by`
                    // already paid for (found on review). A task removed
                    // from the board stays a finding the owner can read.
                    None if board.truncated => GoalLiveness::Unknown(format!(
                        "{goal} is not in a truncated board answer"
                    )),
                    None => GoalLiveness::Unknown(format!(
                        "{goal} is not on the board — removed, or an answer shorter than it said"
                    )),
                }
            }
            GoalRef::Trigger(name) => match trigger(name) {
                TriggerState::Enabled => GoalLiveness::Open,
                TriggerState::Disabled => GoalLiveness::Closed(format!("{goal} is disabled")),
                TriggerState::Missing => GoalLiveness::Closed(format!("{goal} no longer exists")),
                TriggerState::Unreadable(why) => {
                    GoalLiveness::Unknown(format!("{goal} could not be read: {why}"))
                }
            },
            _ => GoalLiveness::NotAssessed,
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
            surface_unread: None,
            workspace: Some(PathBuf::from("/w")),
            goal: None,
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

        // The goal: kept by the scope, required by the match — the same
        // goal matches, another does not, and neither does a run that
        // presents none or one this build cannot name.
        let morning = || Some(goal("trigger:morning"));
        let toward = s(&["shell"]).toward(morning());
        assert_eq!(toward.scope().goal, morning());
        let run = |g: Option<GoalKey>| {
            Situation::of_run(&["shell".into()], Some(Path::new("/w")))
                .on(Some(SessionKind::Tui))
                .toward(g)
        };
        assert!(toward.scope().matches(&run(morning())));
        assert!(!toward.scope().matches(&run(Some(goal("trigger:evening")))));
        assert!(
            !toward.scope().matches(&run(Some(goal("task:morning")))),
            "kind and id together: the same id under another kind is another goal"
        );
        assert!(
            !toward.scope().matches(&run(None)),
            "an absent goal never widens a goal-scoped rule's reach"
        );
        assert!(!toward
            .scope()
            .matches(&run(Some(GoalKey::Unread("dream:morning".into())))));
        // A scope with no goal — a rule mined with none, or from before the
        // key — matches under every goal and under none, as it did.
        assert!(scope.matches(&run(morning())));
        assert!(scope.matches(&run(None)));
        assert!(old.matches(&run(morning())));
        assert!(!scope.is_standing() && !toward.scope().is_standing());
        assert!(
            !Situation::default().toward(morning()).is_standing(),
            "a goal alone is a scope"
        );
    }

    fn goal(g: &str) -> GoalKey {
        GoalKey::Named(g.parse().unwrap())
    }

    /// A stored goal this build cannot name fails closed on every door:
    /// the scope keeps it, it matches no run, it prints verbatim, and it is
    /// written back as it came. Fails on a lenient read that dropped it to
    /// none, which is every goal.
    #[test]
    fn an_unreadable_goal_on_a_scope_matches_nothing_and_round_trips() {
        for (raw, kept) in [
            (r#"{"tools":["shell"],"goal":"dream:x"}"#, "dream:x"),
            (r#"{"tools":["shell"],"goal":"task:"}"#, "task:"),
            (r#"{"tools":["shell"],"goal":7}"#, "7"),
        ] {
            let stored: Situation = serde_json::from_str(raw).unwrap();
            assert_eq!(stored.goal, Some(GoalKey::Unread(kept.into())), "{raw}");
            assert_eq!(stored.scope().goal, stored.goal, "kept: a key");
            for run in [
                Situation::of_run(&["shell".into()], None),
                Situation::of_run(&["shell".into()], None).toward(Some(goal("task:x"))),
                Situation::of_run(&["shell".into()], None)
                    .toward(Some(GoalKey::Unread(kept.into()))),
            ] {
                assert!(!stored.scope().matches(&run), "{raw} must match nothing");
            }
            assert_eq!(stored.key(), format!("shell for {kept}"));
            assert!(stored.describe().contains("cannot name"));
            let back = serde_json::to_string(&stored).unwrap();
            assert!(back.contains(&format!("\"goal\":\"{kept}\"")), "{back}");
        }
        // A named goal round-trips in the one spelling a reference has.
        let named: Situation =
            serde_json::from_str(r#"{"tools":["shell"],"goal":"trigger:morning"}"#).unwrap();
        assert_eq!(named.goal, Some(goal("trigger:morning")));
        assert!(serde_json::to_string(&named)
            .unwrap()
            .contains("\"goal\":\"trigger:morning\""));
        // Absent and null are no goal, and no goal is not written.
        for raw in [
            r#"{"tools":["shell"]}"#,
            r#"{"tools":["shell"],"goal":null}"#,
        ] {
            let none: Situation = serde_json::from_str(raw).unwrap();
            assert_eq!(none.goal, None, "{raw}");
        }
        assert!(!serde_json::to_string(&Situation::default())
            .unwrap()
            .contains("goal"));
    }

    /// Two members toward different goals — or one toward none — share no
    /// goal, so the region, and a widening by intersection, drops the key;
    /// two toward the same goal keep it. The key names the goal last.
    #[test]
    fn a_region_across_goals_drops_the_goal_key() {
        let morning = s(&["shell"]).toward(Some(goal("trigger:morning")));
        let evening = s(&["shell"]).toward(Some(goal("trigger:evening")));
        assert_eq!(Situation::region([&morning, &evening]).scope().goal, None);
        assert_eq!(
            Situation::region([&morning, &s(&["shell"])]).scope().goal,
            None
        );
        assert_eq!(
            Situation::region([&morning, &morning.clone()]).scope().goal,
            Some(goal("trigger:morning"))
        );
        assert_eq!(morning.key(), "shell @ /w on tui for trigger:morning");
        assert_eq!(
            morning.describe(),
            "shell · denial · tui · /w · trigger:morning"
        );
        assert_eq!(
            Situation::recorded(&["shell".into()], "denial", None, None)
                .toward(Some(goal("task:t-budget")))
                .key(),
            "shell for task:t-budget"
        );
    }

    /// A stored scope naming a surface this build cannot read fails closed:
    /// it keeps a key that matches no run, prints as such, and is never
    /// produced by the two construction doors. The session record's lenient
    /// read is untouched. Fails on the lenient read, which dropped the key
    /// and rode the rule on every surface.
    #[test]
    fn an_unreadable_surface_on_a_scope_matches_nothing_and_round_trips() {
        for (raw, kept) in [
            (r#"{"tools":["shell"],"surface":"Tui"}"#, "Tui"),
            (r#"{"tools":["shell"],"surface":7}"#, "7"),
            (r#"{"tools":["shell"],"surface":"hologram"}"#, "hologram"),
        ] {
            let stored: Situation = serde_json::from_str(raw).unwrap();
            assert_eq!(stored.surface, None, "{raw}");
            assert_eq!(stored.surface_unread.as_deref(), Some(kept), "{raw}");
            assert_eq!(
                stored.scope().surface_unread.as_deref(),
                Some(kept),
                "kept: a key, not a mark"
            );
            for run in [
                Situation::of_run(&["shell".into()], None).on(Some(SessionKind::Tui)),
                Situation::of_run(&["shell".into()], None),
            ] {
                assert!(!stored.scope().matches(&run), "{raw} must match nothing");
            }
            assert_eq!(stored.key(), format!("shell on {kept}"));
            // Written back as it came: the build that can name it gets it.
            let back = serde_json::to_string(&stored).unwrap();
            assert!(back.contains(&format!("\"surface\":\"{kept}\"")), "{back}");
        }
        let none: Situation = serde_json::from_str(r#"{"tools":["shell"]}"#).unwrap();
        assert_eq!(none.surface, None, "absent is absent");
        assert_eq!(none.surface_unread, None);
        let tui: Situation =
            serde_json::from_str(r#"{"tools":["shell"],"surface":"tui"}"#).unwrap();
        assert_eq!(tui.surface, Some(SessionKind::Tui));
        assert!(serde_json::to_string(&tui)
            .unwrap()
            .contains("\"surface\":\"tui\""));
        assert!(!serde_json::to_string(&Situation::default())
            .unwrap()
            .contains("surface"));
    }

    /// A corpus mark is not a place: a situation recorded under the test
    /// override scopes with no surface, so it can never pin a rule to a
    /// surface no run presents — the mirror of a front-end focus.
    #[test]
    fn a_mark_kind_is_never_a_scope_key() {
        for k in Situation::MARK_KINDS {
            let marked =
                Situation::recorded(&["shell".into()], "denial", Some(*k), Some(Path::new("/w")));
            assert_eq!(marked.surface, Some(*k), "recorded as it was");
            assert_eq!(marked.scope().surface, None, "never a key");
            assert_eq!(marked.key(), "shell @ /w");
            assert!(marked.scope().matches(
                &Situation::of_run(&["shell".into()], Some(Path::new("/w")))
                    .on(Some(SessionKind::Tui))
            ));
        }
        assert_eq!(
            s(&["shell"]).scope().surface,
            Some(SessionKind::Tui),
            "a real surface stays"
        );
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
    fn a_record_from_before_the_field_loads_and_an_unknown_surface_fails_closed() {
        let old: Situation = serde_json::from_str("{}").unwrap();
        assert_eq!(old, Situation::default());
        let newer: Situation =
            serde_json::from_str(r#"{"tools":["shell"],"surface":"hologram"}"#).unwrap();
        assert_eq!(newer.tools, vec!["shell"]);
        // A surface this build cannot name still costs no record — and on
        // a stored situation it is parked verbatim, a key that matches
        // nothing, never read as `None`, which would be every surface.
        assert_eq!(newer.surface, None);
        assert_eq!(newer.surface_unread.as_deref(), Some("hologram"));
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
    /// trigger or a front-end tool fold into one tally row, two workspaces
    /// or two surfaces make two rows, and standing is the empty key.
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

    /// R34: a goal key is read against the store that owns it. A task is
    /// closed by its board status or by being absent from a whole board; a
    /// trigger by being removed or disabled; and anything a store cannot
    /// answer is unknown — an unreadable or truncated board never reads as
    /// "open", which is the "no rule is dark" answer the ruling forbids.
    #[test]
    fn a_goal_is_read_against_the_store_that_owns_it() {
        let board = Ok(BoardStatuses::of(&serde_json::json!({"items": [
            {"id": "t-open", "status": "next"},
            {"id": "t-done", "status": "done"},
            {"id": "t-dropped", "status": "dropped"},
            {"id": "t-odd", "status": "someday"},
            {"id": "t-nostatus"}
        ]}))
        .unwrap());
        let triggers = |name: &str| match name {
            "morning" => TriggerState::Enabled,
            "evening" => TriggerState::Disabled,
            "broken" => TriggerState::Unreadable("bad toml".into()),
            _ => TriggerState::Missing,
        };
        let live = |g: &str, b: &Result<BoardStatuses, String>| goal(g).liveness(b, &triggers);
        assert_eq!(live("task:t-open", &board), GoalLiveness::Open);
        assert_eq!(
            live("task:t-done", &board),
            GoalLiveness::Closed("task:t-done is done".into())
        );
        assert!(matches!(
            live("task:t-dropped", &board),
            GoalLiveness::Closed(_)
        ));
        assert!(matches!(
            live("task:t-odd", &board),
            GoalLiveness::Unknown(_)
        ));
        // Absent is unknown, not closed: an answer can be short without
        // saying so (`tasks::project_closed_by`'s lesson).
        assert!(
            matches!(live("task:t-gone", &board), GoalLiveness::Unknown(w) if w.contains("not on the board"))
        );
        // A row with no status is not a status: absent from what was read.
        assert!(matches!(
            live("task:t-nostatus", &board),
            GoalLiveness::Unknown(_)
        ));
        // A truncated board cannot say a task is gone.
        let truncated =
            Ok(BoardStatuses::of(&serde_json::json!({"items": [], "truncated": true})).unwrap());
        assert!(matches!(
            live("task:t-gone", &truncated),
            GoalLiveness::Unknown(_)
        ));
        // An unreadable board is its own finding, never open.
        let unread: Result<BoardStatuses, String> = Err("no graph server".into());
        assert!(
            matches!(live("task:t-open", &unread), GoalLiveness::Unknown(w) if w.contains("no graph server"))
        );
        assert!(BoardStatuses::of(&serde_json::json!({"answer": "no"})).is_err());
        // Triggers, read without the board.
        assert_eq!(live("trigger:morning", &unread), GoalLiveness::Open);
        assert!(
            matches!(live("trigger:evening", &unread), GoalLiveness::Closed(w) if w.contains("disabled"))
        );
        assert!(
            matches!(live("trigger:gone", &unread), GoalLiveness::Closed(w) if w.contains("no longer exists"))
        );
        assert!(matches!(
            live("trigger:broken", &unread),
            GoalLiveness::Unknown(_)
        ));
        // Kinds no run is handed as a matched goal, and a parked key.
        assert_eq!(live("charter:rest", &board), GoalLiveness::NotAssessed);
        assert_eq!(
            GoalKey::Unread("dream:x".into()).liveness(&board, &triggers),
            GoalLiveness::NotAssessed
        );
        assert!(goal("task:t-open").needs_board());
        assert!(!goal("trigger:morning").needs_board());
    }

    #[test]
    fn describe_names_the_keys_and_standing_says_so() {
        assert_eq!(s(&["shell"]).describe(), "shell · denial · tui · /w");
        assert_eq!(s(&["shell"]).scope().describe(), "shell · tui · /w");
        assert_eq!(Situation::default().describe(), "everywhere");
    }
}
