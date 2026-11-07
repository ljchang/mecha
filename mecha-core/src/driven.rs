//! A driven run: an outside agent (the driver) running mecha on real work,
//! marked `MECHA_SESSION_KIND=driven` (`docs/DRIVER-DESIGN.md`).
//!
//! The mark alone gives a run nothing new (D1); this module is what it
//! **forces, refuses and records** (D1b, §7), plus the home-side marker a
//! task's setup writes (§2). Everything here narrows: the mark turns
//! surfaces off and refuses starts, and the one thing it may ever *add* — the
//! driven learning store (D2) — needs the marker as a second factor.
//!
//! The CLI applies it (`mecha-cli`'s `setup` and `commands::run`). The parts
//! that decide are pure functions of a config, a sandbox and some paths, so
//! they are tested here without the environment variable, which
//! [`crate::session::SessionKind::test_override`] ignores under `cfg(test)`.

use crate::config::{Config, PermissionMode, TrifectaPolicy};
use crate::sandbox::Sandbox;
use crate::session::SessionKind;
use crate::tool::{Approver, Decision, Tool};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The tools a driven run may register, and no others (§7, "an allowlist,
/// never a sweep"): the builtins whose effects stay inside the jail, and
/// four harness tools a narrowing would otherwise drop with everything else.
/// A tool added to mecha later is out of driven runs until it is named here.
///
/// Sorted, because it is handed to `--tool`'s narrowing as a list and the
/// registry's order is its own (`BTreeMap`); the order here changes nothing
/// a model sees.
pub const TOOLS: &[&str] = &[
    crate::tool::code::NAME,
    "compact",
    "fs_edit",
    "fs_list",
    "fs_read",
    "fs_write",
    "goal_context",
    "shell",
    "skill",
    "todo",
];

/// Is this process marked driven? The one reading of the mark, so every site
/// that forces or refuses on it agrees.
pub fn marked() -> bool {
    SessionKind::test_override() == Some(SessionKind::Driven)
}

/// The stem every refusal of an approval rule's `prompt` carries in a driven
/// run — the rule's own `prompt`, or the `strict_inline_eval` floor. D4
/// counts rows whose transcript holds it as **approval-floor refusals**,
/// apart from model gaps, so it is a constant, never rephrased at a call
/// site, and carried by this door alone.
pub const PROMPT_REFUSED: &str = "a driven run treats an approval rule's `prompt` as `forbid`";

/// The stem of a driven run's refusal of the interlock's `ask`
/// (`Approver::escalate`). Unreachable while the mark forces
/// `trifecta = "block"` and registers no sender; its own stem so that, if it
/// is ever reached, D4 does not count it as an approval-floor refusal.
pub const ESCALATION_REFUSED: &str = "a driven run puts no send in front of a person";

/// The stem of a driven run's refusal of a reach outside its jail
/// (`Approver::escalate_access`): the path jail, which no rewording of the
/// call gets around.
pub const ACCESS_REFUSED: &str = "a driven run is held to its workspace and grants no access";

/// What a driven run forced, recorded on the run beside `levers_off`
/// (`RunConfig::driven`). None of these is a `harness::Lever`, so without
/// this record a later reader could not tell they were in force, and the
/// approver's mode is what decides whether a failed row is a model gap or
/// an approver refusal.
///
/// A wire format. The forced values have **no default**: a record missing
/// one is a record this build cannot read, and it loads as `None` on the run
/// (`RunConfig::driven`'s lenient reader) rather than as a run that forced
/// nothing. Only the lists and the marker, whose empty value is a true
/// reading, default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forced {
    /// `[security] block_sends_after_private`, forced on.
    pub block_sends_after_private: bool,
    /// `private` armed before the first turn: the charter rides in the
    /// system prompt where no taint sees it (§9, R8).
    pub private_armed_at_start: bool,
    /// `[security] trifecta`, forced to `block`; `owner_trifecta` is what the
    /// owner's file said.
    pub trifecta: TrifectaPolicy,
    pub owner_trifecta: TrifectaPolicy,
    /// `[tools] permission_mode`, forced to `allow`; `owner_permission_mode`
    /// is what the owner's file said.
    pub permission_mode: PermissionMode,
    pub owner_permission_mode: PermissionMode,
    /// An approval rule's `prompt` is refused, never put to anyone
    /// ([`PROMPT_REFUSED`]).
    pub prompt_as_forbid: bool,
    /// Whether any `[[rule]]` named `shell`. With none, the
    /// `strict_inline_eval` floor is unreachable and an approval-floor count
    /// of zero means "could not happen", not "did not happen" — a dash is
    /// never a zero.
    pub shell_rule_loaded: bool,
    /// Every `mecha.toml` found in the workspace (and the working directory's
    /// own), none of them merged: a driven run reads no project layer.
    #[serde(default)]
    pub project_configs: Vec<PathBuf>,
    /// The walk for `project_configs` stopped at its bound, so the list may
    /// be short.
    #[serde(default)]
    pub project_configs_truncated: bool,
    /// The owner's `[[subagent]]` profiles, none registered.
    #[serde(default)]
    pub subagents_dropped: Vec<String>,
    /// The task whose live marker names a clone holding this workspace, if
    /// one does. The store load (D2) keys on it; recorded so a run that
    /// carried no driven rules can be told apart from one that had none.
    #[serde(default)]
    pub marker: Option<String>,
}

/// Force the config a driven run is built from, and say what was forced.
///
/// Acts on the owner's global layer alone (the CLI has already refused the
/// project layer), after every flag has been folded in, so nothing later in
/// setup can undo it. The surfaces that are levers (MCP, hooks, grants,
/// the mailbox, `system_read`) are thrown by the CLI on its options, where
/// `levers_off` reads them for the record; this is the half that is not.
pub fn force(cfg: &mut Config) -> Forced {
    let forced = Forced {
        block_sends_after_private: true,
        private_armed_at_start: true,
        trifecta: TrifectaPolicy::Block,
        owner_trifecta: cfg.security.trifecta,
        permission_mode: PermissionMode::Allow,
        owner_permission_mode: cfg.tools.permission_mode,
        prompt_as_forbid: true,
        // Filled in by setup once the policy, the walk and the marker are read.
        shell_rule_loaded: false,
        project_configs: Vec::new(),
        project_configs_truncated: false,
        subagents_dropped: cfg.subagents.iter().map(|p| p.name.clone()).collect(),
        marker: None,
    };
    cfg.security.block_sends_after_private = true;
    cfg.security.trifecta = TrifectaPolicy::Block;
    cfg.tools.permission_mode = PermissionMode::Allow;
    // No subagent, by clearing the profiles rather than by narrowing: a
    // profile naming only allowlisted tools would survive `--tool`, and its
    // own `provider` could answer a subtree on a model that is not the arm's
    // (§7).
    cfg.subagents.clear();
    forced
}

/// Why a driven run with this config, sandbox and workspace must not start,
/// one line each; empty when it may.
///
/// Both refusals are the same rule: a guard that cannot apply stops the run
/// and never falls through.
pub fn refusals(cfg: &Config, sandbox: &Sandbox, workspace: &Path, home: &Path) -> Vec<String> {
    let mut why = Vec::new();
    // With the leak guard forced on, `shell` reports `Egress::Chosen`
    // whenever the sandbox can reach the network, so every `cargo build`
    // would be refused: a wall of harness refusals for D4 to attribute.
    // `Sandbox::preflight` is `Ok` for a disabled sandbox, so nothing else
    // refuses this start.
    if sandbox.can_reach_network() {
        why.push(format!(
            "the sandbox (`[sandbox] kind = \"{}\"`, network {}) can reach the network, and a \
             driven run needs `kind = \"bwrap\"` or `\"docker\"` with `network = false` in the \
             global config",
            cfg.sandbox.kind.as_str(),
            if cfg.sandbox.network { "on" } else { "off" }
        ));
    }
    let home = resolved(home);
    // The marker, the owner's learning store, the transcripts and the global
    // config live under the home. Bound into the jail — writable worst of
    // all — they are the model's to rewrite. Overlap either way is refused:
    // a parent of the home binds all of it, and a path inside it (the
    // marker directory, say) binds the part that matters.
    let overlaps = |p: &Path| {
        let p = resolved(p);
        home.starts_with(&p) || p.starts_with(&home)
    };
    for (set, paths) in [
        ("readable", &cfg.sandbox.readable),
        ("writable", &cfg.sandbox.writable),
    ] {
        for p in paths.iter().filter(|p| overlaps(p)) {
            why.push(format!(
                "`[sandbox] {set}` names {}, which overlaps the mecha home ({})",
                p.display(),
                home.display()
            ));
        }
    }
    // The outbox stays on, as defence in depth (§7), but the marking of a
    // driven draft on the owner's queue is not built. A route over an
    // allowlisted tool would stage every such call into that queue unmarked,
    // so it is refused rather than left to the owner's file never naming one.
    let routed: Vec<&str> = TOOLS
        .iter()
        .copied()
        .filter(|t| cfg.outbox.tools.iter().any(|r| r == t))
        .collect();
    if !routed.is_empty() {
        why.push(format!(
            "`[outbox] tools` routes {}, and a driven run's drafts are not yet marked on the \
             owner's queue",
            routed.join(", ")
        ));
    }
    if overlaps(workspace) {
        why.push(format!(
            "the workspace {} overlaps the mecha home ({}); a task clone lives outside it",
            workspace.display(),
            home.display()
        ));
    }
    why
}

/// The names in `registered` a driven run may not carry.
pub fn unlisted<'a>(registered: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    registered
        .into_iter()
        .filter(|name| !TOOLS.contains(name))
        .collect()
}

/// The most directory entries [`project_configs`] reads before it stops.
const WALK_BOUND: usize = 200_000;

/// Every `mecha.toml` under `workspace`, and `cwd`'s own if `cwd` is
/// elsewhere, with whether the walk hit its bound. `.git` and `target` are
/// skipped (neither holds a tracked config) and symlinks are not followed.
pub fn project_configs(cwd: &Path, workspace: &Path) -> (Vec<PathBuf>, bool) {
    let mut found = Vec::new();
    let mut seen = 0usize;
    let mut stack = vec![workspace.to_path_buf()];
    let mut truncated = false;
    'walk: while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > WALK_BOUND {
                truncated = true;
                break 'walk;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            if kind.is_dir() {
                if name != ".git" && name != "target" {
                    stack.push(entry.path());
                }
            } else if kind.is_file() && name == Config::PROJECT_FILE {
                found.push(entry.path());
            }
        }
    }
    let own = cwd.join(Config::PROJECT_FILE);
    if own.is_file() && !cwd.starts_with(workspace) {
        found.push(own);
    }
    found.sort();
    (found, truncated)
}

/// The approver a driven run answers with.
///
/// `allow` for the run's own calls: inside a jail with no network, no sends,
/// a closed tool allowlist and a clone with no remote, they need no
/// approval, and the owner's `forbid` rules still apply ahead of any
/// approver. Every question for a person — an approval rule's `prompt`, the
/// interlock's `ask`, a reach outside the jail — is refused, because the only
/// one there to answer is the driver, and answering a `prompt` from policy
/// is what that ruling exists to stop. `Blocked`, never `Deny`: no human
/// spoke, so nothing here is mined as a correction.
pub struct DrivenApprover;

impl DrivenApprover {
    /// Each door says what actually stopped the call, with its own stem, so
    /// the model is told the true constraint and D4 can count each apart.
    fn refuse(tool: &dyn Tool, why: &str, stem: &str, next: &str) -> Decision {
        Decision::Blocked(format!(
            "{why} — and {stem}, so `{}` was not run. Nobody is asked in a driven run; {next}",
            tool.name()
        ))
    }
}

#[async_trait]
impl Approver for DrivenApprover {
    async fn approve(&self, _tool: &dyn Tool, _input: &Value) -> Decision {
        Decision::Allow
    }

    async fn permit(&self, _tool: &dyn Tool, _input: &Value) -> Decision {
        Decision::Allow
    }

    async fn consult(&self, tool: &dyn Tool, _input: &Value, why: &str) -> Decision {
        Self::refuse(
            tool,
            why,
            PROMPT_REFUSED,
            "find a way to do this that the rule does not cover.",
        )
    }

    async fn escalate(&self, tool: &dyn Tool, _input: &Value, why: &str) -> Decision {
        Self::refuse(
            tool,
            why,
            ESCALATION_REFUSED,
            "this run sends nothing, so finish the task without it.",
        )
    }

    async fn escalate_access(&self, tool: &dyn Tool, _input: &Value, why: &str) -> Decision {
        Self::refuse(
            tool,
            why,
            ACCESS_REFUSED,
            "work with what is inside the workspace.",
        )
    }
}

/// A task's marker: written under the mecha home by task setup, naming the
/// task clone it covers and the task's deadline (§2). Outside every jail, so
/// the run cannot write it; one per task, so a batch's tasks do not share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub task: String,
    /// Canonical, as written.
    pub clone: PathBuf,
    pub deadline: DateTime<Utc>,
}

/// `<home>/driven/markers`.
pub fn markers_dir(home: &Path) -> PathBuf {
    home.join("driven").join("markers")
}

impl Marker {
    /// Write the marker for `task`, covering `clone` until `deadline`.
    /// Refuses a clone that overlaps the home: the marker must sit outside
    /// every jail it covers.
    pub fn write(home: &Path, task: &str, clone: &Path, deadline: DateTime<Utc>) -> Result<Self> {
        crate::work::valid_producer(task).context("a driven task's name")?;
        let clone = clone
            .canonicalize()
            .with_context(|| format!("task clone {} does not exist", clone.display()))?;
        let home_r = resolved(home);
        anyhow::ensure!(
            !(clone.starts_with(&home_r) || home_r.starts_with(&clone)),
            "task clone {} overlaps the mecha home ({}); a marker must sit outside the jail it covers",
            clone.display(),
            home_r.display()
        );
        let marker = Marker {
            task: task.to_string(),
            clone,
            deadline,
        };
        let dir = markers_dir(home);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{task}.json"));
        let tmp = dir.join(format!(".{task}.json.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(&marker)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(marker)
    }

    /// Remove `task`'s marker. `false` when there was none.
    pub fn remove(home: &Path, task: &str) -> Result<bool> {
        crate::work::valid_producer(task).context("a driven task's name")?;
        let path = markers_dir(home).join(format!("{task}.json"));
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
        }
    }

    /// Every marker under `home`. **An unreadable one is an error**, naming
    /// the file: which workspace it covers is unknown, and unknown is never
    /// clean, because the refusal it feeds is the unsafe direction.
    pub fn all(home: &Path) -> Result<Vec<Marker>> {
        let dir = markers_dir(home);
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
        };
        let mut out = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let marker = std::fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|b| Ok(serde_json::from_slice::<Marker>(&b)?))
                .with_context(|| {
                    format!(
                        "driven marker {} is unreadable, so which workspace it covers is \
                         unknown; remove it with `mecha driven unmark` once its task is done",
                        path.display()
                    )
                })?;
            out.push(marker);
        }
        out.sort_by(|a, b| a.task.cmp(&b.task));
        Ok(out)
    }

    /// Does this marker's clone hold `workspace`?
    pub fn covers(&self, workspace: &Path) -> bool {
        resolved(workspace).starts_with(&self.clone)
    }

    /// Is the marker before its deadline at `now`? Governs the store load
    /// only; the unmarked-run refusal keys on presence.
    pub fn live(&self, now: DateTime<Utc>) -> bool {
        now < self.deadline
    }
}

/// Refuse a run **not** marked driven whose workspace a marker covers,
/// expired or not.
///
/// Every surface the mark removes is gated on the mark alone, so a driven
/// task whose mark was lost would run with the owner's servers, hooks,
/// grants and mailbox attached. A task that overruns, or a stalled run on its
/// second attempt, is exactly when a mark is most likely to have been lost,
/// which is why the deadline does not lift this.
pub fn refuse_unmarked(home: &Path, workspace: &Path) -> Result<()> {
    if let Some(m) = Marker::all(home)?.into_iter().find(|m| m.covers(workspace)) {
        anyhow::bail!(
            "workspace {} is inside driven task `{}`'s clone ({}), and this run is not marked \
             driven; a driven task runs with `MECHA_SESSION_KIND=driven`. If the task is \
             finished, `mecha driven unmark {}`.",
            workspace.display(),
            m.task,
            m.clone.display(),
            m.task
        );
    }
    Ok(())
}

/// The task whose **live** marker covers `workspace`, for a marked run.
pub fn live_task(home: &Path, workspace: &Path, now: DateTime<Utc>) -> Result<Option<String>> {
    Ok(Marker::all(home)?
        .into_iter()
        .find(|m| m.covers(workspace) && m.live(now))
        .map(|m| m.task))
}

fn resolved(p: &Path) -> PathBuf {
    crate::work::as_it_would_resolve(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{Backend, SandboxConfig};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mecha-driven-{name}-{}-{}",
            std::process::id(),
            crate::session::Session::new_id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn sandbox(kind: Backend, network: bool) -> Sandbox {
        Sandbox::new(SandboxConfig {
            kind,
            network,
            ..SandboxConfig::default()
        })
    }

    #[test]
    fn force_turns_on_the_leak_guard_and_block_and_allow_and_drops_subagents() {
        let mut cfg = Config::default();
        cfg.security.trifecta = TrifectaPolicy::Ask;
        cfg.tools.permission_mode = PermissionMode::Ask;
        cfg.subagents = toml::from_str::<Config>(
            "[[subagent]]\nname = \"research\"\ndescription = \"d\"\ntools = [\"fs_read\"]\n",
        )
        .unwrap()
        .subagents;
        assert_eq!(cfg.subagents.len(), 1, "fixture parses one profile");
        let forced = force(&mut cfg);
        assert!(cfg.security.block_sends_after_private);
        assert_eq!(cfg.security.trifecta, TrifectaPolicy::Block);
        assert_eq!(cfg.tools.permission_mode, PermissionMode::Allow);
        assert!(cfg.subagents.is_empty());
        assert_eq!(forced.owner_trifecta, TrifectaPolicy::Ask);
        assert_eq!(forced.owner_permission_mode, PermissionMode::Ask);
        assert_eq!(forced.subagents_dropped, vec!["research".to_string()]);
        assert!(forced.prompt_as_forbid && forced.private_armed_at_start);
    }

    #[test]
    fn a_sandbox_that_can_reach_the_network_refuses_the_start() {
        let home = tmp("home");
        let ws = tmp("ws");
        let cfg = Config::default();
        for (kind, network, refused) in [
            (Backend::None, false, true),
            (Backend::Landlock, false, true),
            (Backend::Bwrap, true, true),
            (Backend::Docker, true, true),
            (Backend::Bwrap, false, false),
            (Backend::Docker, false, false),
        ] {
            let why = refusals(&cfg, &sandbox(kind, network), &ws, &home);
            assert_eq!(
                !why.is_empty(),
                refused,
                "{kind:?} network={network}: {why:?}"
            );
        }
    }

    #[test]
    fn a_bind_overlapping_the_home_in_either_direction_refuses_the_start() {
        let root = tmp("bind");
        let home = root.join("home");
        std::fs::create_dir_all(home.join("driven")).unwrap();
        let ws = tmp("ws");
        let toolchain = tmp("toolchain");
        let ok = sandbox(Backend::Bwrap, false);
        for (set, path, refused) in [
            ("readable", toolchain.clone(), false),
            ("readable", root.clone(), true),
            ("readable", home.join("driven"), true),
            ("writable", home.clone(), true),
            ("writable", home.join("not-yet").join(".."), true),
        ] {
            let mut cfg = Config::default();
            match set {
                "readable" => cfg.sandbox.readable = vec![path.clone()],
                _ => cfg.sandbox.writable = vec![path.clone()],
            }
            let why = refusals(&cfg, &ok, &ws, &home);
            assert_eq!(
                !why.is_empty(),
                refused,
                "{set} {}: {why:?}",
                path.display()
            );
        }
        // An outbox route over an allowlisted tool: its drafts would stage
        // into the owner's queue unmarked.
        let mut cfg = Config::default();
        cfg.outbox.tools = vec!["mail__mail_send".into()];
        assert!(refusals(&cfg, &ok, &ws, &home).is_empty());
        cfg.outbox.tools.push("fs_write".into());
        let why = refusals(&cfg, &ok, &ws, &home);
        assert!(why.len() == 1 && why[0].contains("fs_write"), "{why:?}");
        // And the workspace itself, inside the home.
        let why = refusals(&Config::default(), &ok, &home.join("driven"), &home);
        assert_eq!(why.len(), 1, "{why:?}");
    }

    #[test]
    fn only_the_named_tools_pass() {
        let names = [
            "fs_read",
            "shell",
            "http_fetch",
            "web_search",
            "recall",
            "kg__search",
        ];
        assert_eq!(
            unlisted(names),
            vec!["http_fetch", "web_search", "recall", "kg__search"]
        );
        let mut sorted = TOOLS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, TOOLS, "kept sorted");
    }

    #[test]
    fn project_configs_are_found_anywhere_but_git_and_target() {
        let ws = tmp("walk");
        for dir in ["a/b", ".git", "target/x"] {
            std::fs::create_dir_all(ws.join(dir)).unwrap();
        }
        for f in [
            "mecha.toml",
            "a/b/mecha.toml",
            ".git/mecha.toml",
            "target/x/mecha.toml",
        ] {
            std::fs::write(ws.join(f), "").unwrap();
        }
        let (found, truncated) = project_configs(&ws, &ws);
        assert!(!truncated);
        assert_eq!(
            found,
            vec![ws.join("a/b/mecha.toml"), ws.join("mecha.toml")]
        );
        // The working directory's own, when it is elsewhere.
        let cwd = tmp("cwd");
        std::fs::write(cwd.join("mecha.toml"), "").unwrap();
        let (found, _) = project_configs(&cwd, &ws);
        assert!(found.contains(&cwd.join("mecha.toml")), "{found:?}");
    }

    #[test]
    fn a_marker_refuses_an_unmarked_run_in_its_clone_expired_or_not() {
        let home = tmp("mhome");
        let clone = tmp("clone");
        std::fs::create_dir_all(clone.join("sub")).unwrap();
        let elsewhere = tmp("elsewhere");
        // Nothing written: nothing refused.
        refuse_unmarked(&home, &clone).unwrap();
        let past = Utc::now() - chrono::Duration::hours(1);
        Marker::write(&home, "t-1", &clone, past).unwrap();
        for ws in [clone.clone(), clone.join("sub")] {
            let e = refuse_unmarked(&home, &ws).unwrap_err().to_string();
            assert!(e.contains("t-1"), "{e}");
        }
        refuse_unmarked(&home, &elsewhere).unwrap();
        // Expired: refuses, but loads nothing.
        assert_eq!(live_task(&home, &clone, Utc::now()).unwrap(), None);
        Marker::write(
            &home,
            "t-1",
            &clone,
            Utc::now() + chrono::Duration::hours(1),
        )
        .unwrap();
        assert_eq!(
            live_task(&home, &clone.join("sub"), Utc::now()).unwrap(),
            Some("t-1".into())
        );
        assert_eq!(live_task(&home, &elsewhere, Utc::now()).unwrap(), None);
        assert!(Marker::remove(&home, "t-1").unwrap());
        assert!(!Marker::remove(&home, "t-1").unwrap());
        refuse_unmarked(&home, &clone).unwrap();
    }

    #[test]
    fn an_unreadable_marker_is_an_error_and_a_clone_in_the_home_is_refused() {
        let home = tmp("bad");
        std::fs::create_dir_all(markers_dir(&home)).unwrap();
        std::fs::write(markers_dir(&home).join("x.json"), "{not json").unwrap();
        let ws = tmp("ws2");
        let e = format!("{:#}", refuse_unmarked(&home, &ws).unwrap_err());
        assert!(e.contains("x.json") && e.contains("unknown"), "{e}");
        std::fs::remove_file(markers_dir(&home).join("x.json")).unwrap();
        let inside = home.join("clone");
        std::fs::create_dir_all(&inside).unwrap();
        assert!(Marker::write(&home, "t", &inside, Utc::now()).is_err());
        assert!(Marker::write(&home, "Bad Name", &ws, Utc::now()).is_err());
    }

    #[test]
    fn the_record_round_trips_and_an_unknown_shape_is_none_on_the_run() {
        let mut cfg = Config::default();
        let forced = force(&mut cfg);
        let back: Forced = serde_json::from_str(&serde_json::to_string(&forced).unwrap()).unwrap();
        assert_eq!(back, forced);
        let run: crate::session::RunConfig =
            serde_json::from_value(serde_json::json!({"driven": {"trifecta": "sideways"}}))
                .unwrap();
        assert_eq!(run.driven, None);
        // A record missing a forced value is unknown, never "forced nothing".
        let mut partial = serde_json::to_value(&forced).unwrap();
        partial.as_object_mut().unwrap().remove("prompt_as_forbid");
        let run: crate::session::RunConfig =
            serde_json::from_value(serde_json::json!({ "driven": partial })).unwrap();
        assert_eq!(run.driven, None);
        // The lists and the marker may be absent: empty is a true reading.
        let mut lean = serde_json::to_value(&forced).unwrap();
        for k in [
            "project_configs",
            "project_configs_truncated",
            "subagents_dropped",
            "marker",
        ] {
            lean.as_object_mut().unwrap().remove(k);
        }
        let back: Forced = serde_json::from_value(lean).unwrap();
        assert_eq!(back, forced);
    }

    struct Named(&'static str);
    #[async_trait]
    impl Tool for Named {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            ""
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }
        async fn call(
            &self,
            _input: Value,
            _ctx: &crate::tool::ToolCtx,
        ) -> Result<crate::tool::ToolOutput> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn the_approver_allows_the_runs_own_calls_and_refuses_every_question() {
        let a = DrivenApprover;
        let t = Named("shell");
        let input = serde_json::json!({});
        assert!(matches!(a.approve(&t, &input).await, Decision::Allow));
        assert!(matches!(a.permit(&t, &input).await, Decision::Allow));
        let stems = [PROMPT_REFUSED, ESCALATION_REFUSED, ACCESS_REFUSED];
        for (d, stem) in [
            (a.consult(&t, &input, "rule says ask").await, PROMPT_REFUSED),
            (
                a.escalate(&t, &input, "interlock").await,
                ESCALATION_REFUSED,
            ),
            (
                a.escalate_access(&t, &input, "outside").await,
                ACCESS_REFUSED,
            ),
        ] {
            match d {
                Decision::Blocked(why) => {
                    // Its own stem, and no other door's: D4 counts each apart.
                    for other in stems {
                        assert_eq!(why.contains(other), other == stem, "{why}");
                    }
                }
                other => panic!("{other:?}"),
            }
        }
    }
}
