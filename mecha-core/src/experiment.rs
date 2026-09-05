//! Experiments: a designed comparison over a chosen set of runs — the unit
//! larger than a run that no other store here has (`docs/EXPERIMENT-DESIGN.md`,
//! Part I §3–§4 and Part II §14–§15, §18).
//!
//! What lives here is the store and the pure parts of the runner: the
//! **manifest** (the design, written before anything runs — arms, the
//! control, each treatment arm's falsifiable prediction, the tasks, the
//! seeds, the split seed), the **trial** record (one row per
//! arm × task × seed × repetition), the **isolated home** every trial runs
//! in, the **child invocation** that expresses an arm as a config file and
//! a set of `--no-*` flags, and the **judge** that pairs a treatment arm
//! against the control through `candidate::judge_slices` with a uniformly
//! drawn holdout. The process spawning is `mecha exp`'s, in the CLI, on D3:
//! every actor is its own `mecha` process, and this crate knows nothing
//! about any binary.
//!
//! Three rules carried over from the other stores, restated once because
//! this is the sixth store and the expensive mistake is inventing a
//! seventh convention:
//!
//! - **One pretty JSON per trial, temp-sibling-and-rename**, so a reader
//!   never sees a torn row.
//! - **Closed enums on an append-only store are wire formats**: a trial
//!   status this build does not know reads as unknown, never as a failed
//!   file.
//! - **An arm may only vary the closed set.** Levers by name from
//!   [`crate::harness::Lever`], knobs by `KEY=VALUE` through
//!   [`crate::harness::parse_change`]. An unknown lever name in a manifest
//!   is a load error, never a skipped line (D14) — and `approval_rules` is
//!   refused outright, because a `forbid` is the operator's standing word and
//!   only `mecha eval`'s fixture workspaces justify lifting it.
//!
//! And one rule that is this store's own (D12): **isolation is the whole
//! store, not the mailbox.** Every trial runs with `MECHA_HOME` pointing at a
//! home under the experiment directory, whose `config.toml` *is* the arm, so
//! nothing about an arm is ambient — and the runner refuses a home that is,
//! or contains, the real one. A rule learned inside a trial that landed in
//! `~/.mecha/learning/` would ride every real run's cached prefix from then
//! on, a longer half-life than any injection the interlock guards against.

use crate::candidate::{ChangeClass, Judgement, Metric};
use crate::harness::Lever;
use crate::session::RunStats;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ─── The manifest ───────────────────────────────────────────────────────────

/// The design, written before the run. Loaded through [`Manifest::load`],
/// which is where every rule below is enforced; a `Manifest` value in hand
/// is one that passed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `single` or `lifetime` (Part II §14): one run per arm × task × seed
    /// × repetition, or one home per arm × seed × repetition walked through
    /// the task sequence with the loop's stages between (`schedule`).
    #[serde(default)]
    pub kind: TrialKind,
    /// The arm every treatment arm is paired against. Must name an arm, and
    /// that arm carries no prediction: the control predicts nothing.
    ///
    /// **`None` is a measurement, not a comparison**: one or more arms, none
    /// carrying a prediction, nothing for the gate to rule on. Every plain
    /// `mecha eval` records one — a scorecard is a one-arm experiment — and
    /// a bake-off of several models is one with several arms, each read for
    /// what it measured rather than paired against anything.
    #[serde(default)]
    pub control: Option<String>,
    pub arms: BTreeMap<String, Arm>,
    pub tasks: Tasks,
    /// Recorded per trial and set on the child's provider. Empty means one
    /// unseeded run per task — the server chooses, and the trial says so.
    #[serde(default)]
    pub seeds: Vec<u64>,
    #[serde(default = "one")]
    pub repetitions: u32,
    /// The holdout draw's seed (`sample::take_uniform`). Fixed in the
    /// manifest, before any trial runs, on `candidate.rs`'s rule: a holdout
    /// drawn after looking at the trials is the multiple-comparisons trap
    /// it exists to close.
    pub split_seed: u64,
    /// Every `holdout_in`-th pair is held out, at least
    /// `candidate::MIN_HOLDOUT_PAIRS`.
    #[serde(default = "three")]
    pub holdout_in: u64,
    /// A `lifetime`'s loop stages between tasks (Part II §14): after every
    /// task `reflect`, after every fifth `validate` then `learn --auto`
    /// then `rules propose-retirements --apply` (validate measures before
    /// learn consumes; the brake after), after every tenth
    /// `harness ruminate`, by default. Sequence and
    /// schedule live here, on the design, so the stage order a lifetime ran
    /// under is on the record and never in a script. A `single` manifest
    /// carries none.
    #[serde(default)]
    pub schedule: Schedule,
    /// A `lifetime`'s principal (Part II §16, §21.1): an executable the
    /// driver calls before and after each task with the trial's state on
    /// stdin, answering with the owner's verbs to run and the refusals to
    /// script. The driver runs the verbs — through `mecha` against the
    /// trial home, from a closed set — and records every act on the
    /// lifetime's ledger, so the principal is pure and the record is the
    /// driver's. A `single` refuses one.
    #[serde(default)]
    pub principal: Option<Principal>,
    /// The fixture servers a trial home carries **instead of** the
    /// operator's (Part II §17 item 4, §21.1): when this names any, the
    /// rendered `[[mcp]]` is exactly this list and every live server is
    /// dropped, for every arm. That closed world is what lets the principal
    /// release a draft and close a board task — the two owner channels
    /// gated off it until now, because a `full` arm carried the operator's
    /// live mail and graph into the home and the store was isolated while
    /// the effect was not. Each server persists under the home
    /// (`fixtures/<name>/`, handed to it as `MECHA_FIXTURE_DIR`), seeded
    /// once from the manifest's `seed` directory, so a lifetime's board and
    /// mailbox are the dataset and D12 holds for them too.
    #[serde(default)]
    pub fixtures: Fixtures,
}

/// What the trial home carries in place of the operator's world: fixture
/// MCP servers, and optionally the charter. `[fixtures]` on the manifest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fixtures {
    /// A charter for the trial home, written over the one `seed_home`
    /// copies from the real home before every task. The synthetic
    /// assistant home's owner has their own standing priorities; the
    /// operator's are not the design. The owner's rule survives: a file the
    /// experimenter wrote, no model authors a line. **Optional on
    /// purpose**: a fixture world with no charter runs under the
    /// operator's, which is the design when the question is how *this*
    /// owner's priorities fare against a controlled world — the one case
    /// refused is a fixture charter over the operator's live servers.
    pub charter: Option<PathBuf>,
    /// The servers. Named like `[[mcp]]` entries, minus what a fixture never
    /// needs (an `env`, a passthrough, a sandbox), plus a `seed`.
    pub mcp: Vec<FixtureServer>,
    /// The outbox route of this world: the fixture tools whose calls are
    /// **staged, not executed** (`[outbox] tools`' meaning, on the closed
    /// world's names). **Spelled, never inherited**: the operator's route
    /// names live tools that are not in a fixture home, and inheriting it
    /// made whether a fixture send was staged — the precondition for the
    /// release channel — depend on the operator's config happening to spell
    /// `mail__mail_send`; on a machine with the default empty route every
    /// send executed unrouted into the fixture store, no draft ever pended,
    /// and every position read clean (found on review). So a manifest that
    /// names fixtures must name the route too — `[]` for a world with no
    /// staged sink, said out loud — and each name must be a prefixed
    /// fixture's tool, since a release is permitted only for those.
    pub outbox_tools: Option<Vec<String>>,
}

/// One fixture MCP server on the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureServer {
    /// The `[[mcp]] name` the home's config carries — and, unless
    /// `prefix_tools = false`, the prefix on every tool it exposes, which
    /// is how the driver tells a fixture's tool from a builtin's when the
    /// principal releases a draft. No `__` in a name: it is the separator.
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub prefix_tools: Option<bool>,
    #[serde(default)]
    pub capabilities: crate::config::CapabilityOverride,
    /// A directory copied into the server's store under the home when that
    /// store is first created — the board's tasks, the mailbox's threads.
    /// Copied once: what a lifetime's runs did to the store is the record.
    #[serde(default)]
    pub seed: Option<PathBuf>,
}

/// The variable a fixture server reads its store directory from — under
/// the trial home, set by the driver, never by the manifest.
pub const FIXTURE_DIR_ENV: &str = "MECHA_FIXTURE_DIR";

/// The marker a seeded fixture store carries, written after the seed
/// landed; its absence means the store is torn and is rebuilt.
pub const FIXTURE_SEEDED: &str = ".seeded";

/// Where a fixture server's state lives in a trial home.
pub fn fixture_dir(home: &Path, name: &str) -> PathBuf {
    home.join("fixtures").join(name)
}

/// Canonicalise every argument that is a relative path to a file that
/// exists under `base` — the manifest's paths are written against the
/// checkout `mecha exp run` is started from, and a server is spawned from
/// the run's workspace, where `eval/fixtures/board_server.py` names
/// nothing (eval's `--mcp-file` learnt this the same way). Anything else is
/// left alone: a flag, a name on `PATH`, a path that is not there yet. The
/// command itself (`argv[0]`) is resolved only when it is spelled as a
/// path — `eval/fixtures/x.py`, never a bare `python3` — so a checkout
/// that happened to hold a file named like the interpreter could not
/// rewrite the executable (found on review).
pub fn resolve_file_args(argv: &mut [String], base: &Path) {
    for (i, a) in argv.iter_mut().enumerate() {
        let p = Path::new(a.as_str());
        if i == 0 && p.components().count() < 2 {
            continue;
        }
        if p.is_relative() {
            let joined = base.join(p);
            if joined.is_file() {
                if let Ok(c) = joined.canonicalize() {
                    *a = c.to_string_lossy().into_owned();
                }
            }
        }
    }
}

impl Fixtures {
    pub fn is_empty(&self) -> bool {
        self.mcp.is_empty()
    }

    /// The server names, sorted — the hash term and the principal's view.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.mcp.iter().map(|s| s.name.clone()).collect();
        v.sort();
        v
    }

    fn validate(&self) -> Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        for s in &self.mcp {
            anyhow::ensure!(!s.name.is_empty(), "a fixture server needs a name");
            anyhow::ensure!(
                !s.name.contains("__"),
                "fixture server `{}`: `__` separates a server's name from its tools' and cannot be in the name",
                s.name
            );
            anyhow::ensure!(
                seen.insert(s.name.as_str()),
                "fixture server `{}` is named twice",
                s.name
            );
            anyhow::ensure!(
                !s.command.is_empty(),
                "fixture server `{}` names no command",
                s.name
            );
        }
        if self.charter.is_some() {
            anyhow::ensure!(
                !self.mcp.is_empty(),
                "`[fixtures] charter` without a fixture server: a home whose world is the operator's live servers is not a fixture home, and a fixture charter over it would appraise the wrong owner"
            );
        }
        match &self.outbox_tools {
            None if !self.mcp.is_empty() => anyhow::bail!(
                "`[fixtures]` names servers but no `outbox_tools`: the route is not inherited from the operator's config (its names are live tools that are not in this world), so name the fixture tools whose calls are staged — or `outbox_tools = []` for a world with no staged sink"
            ),
            Some(_) if self.mcp.is_empty() => anyhow::bail!(
                "`[fixtures] outbox_tools` without a fixture server: the route is the fixture world's, and this manifest has none"
            ),
            Some(tools) => {
                let prefixed: Vec<&str> = self
                    .mcp
                    .iter()
                    .filter(|s| s.prefix_tools != Some(false))
                    .map(|s| s.name.as_str())
                    .collect();
                for tool in tools {
                    anyhow::ensure!(
                        tool.split_once("__")
                            .is_some_and(|(server, rest)| !rest.is_empty() && prefixed.contains(&server)),
                        "`[fixtures] outbox_tools` names `{tool}`, which is not a prefixed fixture server's tool (prefixed fixtures: {}); an unprefixed server's tool cannot be told from a builtin, and a release is permitted only for a fixture's",
                        if prefixed.is_empty() {
                            "none".to_string()
                        } else {
                            prefixed.join(", ")
                        }
                    );
                }
            }
            None => {}
        }
        Ok(())
    }

    /// The route, as the rendered config carries it: the manifest's, or
    /// nothing. Only meaningful once `validate` passed.
    pub fn routed(&self) -> &[String] {
        self.outbox_tools.as_deref().unwrap_or(&[])
    }

    /// The `[[mcp]]` entries the home's config carries: every fixture, each
    /// with its store directory under the home created (and seeded, the
    /// first time) and handed over as [`FIXTURE_DIR_ENV`]; no passthrough,
    /// no sandbox, no inline secret. `base` resolves the manifest's
    /// relative paths. `fresh` drops and re-seeds every store first — a
    /// `single`'s rule, whose per-arm home is shared by every trial of the
    /// arm, so without it trial 2 read the mailbox trial 1 edited under the
    /// same condition hash (found on review); a lifetime never passes it,
    /// since what one task left is what the next starts from.
    ///
    /// **Seeding fails closed.** The store is built in a temporary sibling,
    /// marked [`FIXTURE_SEEDED`] after the copy, and renamed into place, and
    /// "already seeded" is the marker, not the directory: the first cut
    /// created the directory before the seed and keyed the skip on its
    /// existence, so a seed that failed — a wrong cwd, a partial copy — left
    /// an empty store the next render treated as seeded, and every later
    /// position ran against an empty board with one stderr line saying so
    /// (found on review). A directory without the marker is torn and is
    /// rebuilt.
    pub fn render(
        &self,
        home: &Path,
        base: &Path,
        fresh: bool,
    ) -> Result<Vec<crate::config::McpServerConfig>> {
        let mut out = Vec::with_capacity(self.mcp.len());
        for s in &self.mcp {
            let dir = fixture_dir(home, &s.name);
            let seeded = dir.join(FIXTURE_SEEDED).exists();
            if dir.exists() && (fresh || !seeded) {
                std::fs::remove_dir_all(&dir)
                    .with_context(|| format!("clearing {}", dir.display()))?;
            }
            if !dir.exists() {
                let parent = dir.parent().expect("under the home");
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
                let staging = parent.join(format!(".{}.seeding-{}", s.name, uuid::Uuid::new_v4()));
                let built: Result<()> = (|| {
                    std::fs::create_dir_all(&staging)?;
                    if let Some(seed) = &s.seed {
                        let seed = base.join(seed);
                        anyhow::ensure!(
                            seed.is_dir(),
                            "fixture server `{}`: seed {} is not a directory",
                            s.name,
                            seed.display()
                        );
                        copy_tree(&seed, &staging).with_context(|| {
                            format!("seeding {} into {}", seed.display(), dir.display())
                        })?;
                    }
                    std::fs::write(staging.join(FIXTURE_SEEDED), b"seeded by mecha exp\n")?;
                    std::fs::rename(&staging, &dir)
                        .with_context(|| format!("placing {}", dir.display()))?;
                    Ok(())
                })();
                if let Err(e) = built {
                    let _ = std::fs::remove_dir_all(&staging);
                    return Err(e);
                }
            }
            let mut argv = vec![s.command.clone()];
            argv.extend(s.args.iter().cloned());
            resolve_file_args(&mut argv, base);
            let command = argv.remove(0);
            let mut env = BTreeMap::new();
            env.insert(
                FIXTURE_DIR_ENV.to_string(),
                dir.to_string_lossy().into_owned(),
            );
            out.push(crate::config::McpServerConfig {
                name: s.name.clone(),
                command,
                args: argv,
                env,
                env_passthrough: Vec::new(),
                sandbox: false,
                network: None,
                prefix_tools: s.prefix_tools,
                capabilities: s.capabilities,
                disabled: false,
            });
        }
        Ok(out)
    }

    /// Put the fixtures into a rendered trial config: the `[[mcp]]` list
    /// becomes exactly the fixtures, so no live server of the operator's
    /// reaches the home, and the outbox route becomes exactly
    /// `outbox_tools` — the operator's routed names are live tools with
    /// nothing behind them here, and the publish set is emptied because no
    /// fixture publishes. A manifest with none leaves the config alone —
    /// the operator's posture travels, as before.
    pub fn apply(
        &self,
        config: &mut crate::config::Config,
        home: &Path,
        base: &Path,
        fresh: bool,
    ) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        config.mcp = self.render(home, base, fresh)?;
        config.outbox.tools = self.routed().to_vec();
        config.outbox.publish_tools.clear();
        Ok(())
    }

    /// Write the fixture charter over the home's, when the manifest names
    /// one. Idempotent, and run before every task: nothing in a home edits
    /// a charter (no stage does, no model can), so the design's file is
    /// the file.
    pub fn apply_charter(&self, home: &Path, base: &Path) -> Result<()> {
        let Some(path) = &self.charter else {
            return Ok(());
        };
        let from = base.join(path);
        let text = std::fs::read_to_string(&from)
            .with_context(|| format!("reading the fixture charter {}", from.display()))?;
        // Parsed before it is written: an unreadable charter degrades a run
        // to un-chartered with one stderr line, which would measure the
        // wrong thing quietly (found on §11.1's containment 7).
        crate::charter::Charter::parse(&text)
            .with_context(|| format!("the fixture charter {} does not parse", from.display()))?;
        write_atomic(&home.join("charter.toml"), text.as_bytes())
    }
}

/// The principal's contract, on `hooks.rs`'s shape: a command at a
/// lifecycle point, JSON in and out, fail-closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    /// The executable and its arguments. Runs from the lifetime's scratch
    /// workspace with the run child's environment allowlist, so name it by
    /// an absolute path or a command on `PATH`.
    pub command: Vec<String>,
    /// Bounds the principal's answer and, separately, each verb the driver
    /// runs for it — a verb that resumes the parked run is a model run
    /// with no ceiling of its own.
    #[serde(default = "principal_timeout")]
    pub timeout_secs: u64,
}

fn principal_timeout() -> u64 {
    600
}

/// Where in a position the principal is being asked to act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalPoint {
    /// Before the task's run: the principal may script refusals for it.
    BeforeTask,
    /// After the task ran and was graded, before the loop's stages: the
    /// principal judges what the run left — drafts, questions — and closes
    /// what gold closes.
    AfterTask,
    /// A point this build cannot name, read off a ledger a later build
    /// wrote: the line stays readable. Every unknown point shares this one
    /// identity in `stage_health`, so two points a later build adds at one
    /// position read as one pair to this build — a limit, stated rather
    /// than mechanised, since it bites only an older build on a newer
    /// ledger. Never constructed here, never called at.
    #[serde(other)]
    Unknown,
}

impl PrincipalPoint {
    pub fn as_str(self) -> &'static str {
        match self {
            PrincipalPoint::BeforeTask => "before_task",
            PrincipalPoint::AfterTask => "after_task",
            PrincipalPoint::Unknown => "unknown",
        }
    }
}

/// What the driver hands the principal on stdin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrincipalInput {
    pub point: PrincipalPoint,
    pub experiment: String,
    pub lifetime: String,
    pub arm: String,
    pub position: u32,
    pub home: PathBuf,
    pub workspace: PathBuf,
    /// The eval case at this position, whole: its prompt, its
    /// expectations (a `verify` command is gold), its tags.
    pub case: crate::eval::EvalCase,
    /// After the task: the graded row. `None` before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial: Option<Trial>,
    /// What the run left staged in the trial home's outbox, still pending.
    #[serde(default)]
    pub pending_outbox: Vec<crate::outbox::OutboxItem>,
    /// Questions the run parked and nobody has answered.
    #[serde(default)]
    pub open_questions: Vec<crate::questions::Question>,
    /// The fixture servers this arm's verbs can reach, by name — the
    /// manifest's, or none when the arm has MCP off. A principal that
    /// releases or closes without one on this list is asking the driver
    /// for a verb it will refuse.
    #[serde(default)]
    pub fixtures: Vec<String>,
}

/// What the principal answers with.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalOutput {
    /// The owner's verbs to run, as `mecha` argv after the binary — from
    /// the closed set `allowed_verb` names, in order.
    #[serde(default)]
    pub acts: Vec<PrincipalRequest>,
    /// Refusals to script for the task about to run (`before_task`):
    /// written to the trial home's denials file, which the run child's
    /// approver reads ahead of its own answer.
    #[serde(default)]
    pub deny: Vec<crate::tool::DenialRule>,
}

/// One verb the principal asks for: what it answers with. Strict — an
/// exit code or a verdict on it is the driver's alone, and a principal
/// that claims one is off the contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrincipalRequest {
    pub verb: Vec<String>,
    #[serde(default)]
    pub reason: String,
}

/// One verb as the ledger records it: the request, plus how the driver's
/// run of it went. A separate type from the request on purpose — a single
/// type that refused the driver's own fields on read made every line with
/// an act unreadable, torn on the ledger and invisible to `stage_health`
/// (found on review).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrincipalAct {
    pub verb: Vec<String>,
    #[serde(default)]
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
}

impl From<PrincipalRequest> for PrincipalAct {
    fn from(r: PrincipalRequest) -> Self {
        PrincipalAct {
            verb: r.verb,
            reason: r.reason,
            exit_code: None,
            ok: None,
        }
    }
}

/// The owner's verbs a principal may run, by their leading words — the
/// channels §16 names, and nothing that writes a session, a reflection or
/// a rule. A verb outside the set is the principal's error, recorded and
/// never run. Two of them reach a server ([`SERVER_VERBS`]) and are
/// permitted only under a manifest that names fixture servers.
pub const PRINCIPAL_VERBS: [&[&str]; 8] = [
    &["tasks", "set"],
    &["tasks", "steer"],
    &["tasks", "stop"],
    &["outbox", "approve"],
    &["outbox", "reject"],
    &["outbox", "edit"],
    &["questions", "answer"],
    &["questions", "abandon"],
];

/// The verbs whose effect leaves the home's stores for a server's:
/// `outbox approve` executes the routed tool for real, and `tasks set`
/// writes the board, which is the graph's over MCP. Under a manifest with
/// no fixtures a `full` arm carries the operator's live servers into the
/// trial home — the store is isolated, the effect is not — so a principal
/// that released would send from a real account and one that closed would
/// close a real task, which in this repo crosses a human structurally.
/// Under a manifest that names fixtures the home's `[[mcp]]` is exactly
/// those, and the effect lands in `fixtures/<name>/` under the home.
pub const SERVER_VERBS: [&[&str]; 2] = [&["tasks", "set"], &["outbox", "approve"]];

fn leads_with(verb: &[String], lead: &[&str]) -> bool {
    verb.len() >= lead.len() && lead.iter().zip(verb).all(|(a, b)| a == b)
}

pub fn reaches_a_server(verb: &[String]) -> bool {
    SERVER_VERBS.iter().any(|lead| leads_with(verb, lead))
}

/// Whether the driver may run this verb for the principal: the shape
/// ([`allowed_verb`]) and, for a verb that reaches a server, that the
/// manifest names fixture servers. The reason on refusal is the ledger's.
pub fn permitted_verb(verb: &[String], fixtures_named: bool) -> std::result::Result<(), String> {
    if !allowed_verb(verb) {
        return Err(format!(
            "`{}` is not an owner's verb the principal may run",
            verb.join(" ")
        ));
    }
    if reaches_a_server(verb) && !fixtures_named {
        return Err(format!(
            "`{}` reaches a server, and this manifest names no fixture servers — a release or a closure would land on the operator's live ones",
            verb.join(" ")
        ));
    }
    Ok(())
}

/// Whether a draft's tool is one a fixture server exposes, by the
/// `<name>__` prefix the registry gives every tool of a prefixed server.
/// A builtin (`http_fetch`, `web_search`) and an unprefixed server's tool
/// carry none, so a release of either is refused: the driver cannot tell
/// where the effect lands, and unknown is never clean.
pub fn release_target_is_fixture(tool: &str, fixtures: &[String]) -> bool {
    match tool.split_once("__") {
        Some((server, rest)) => !rest.is_empty() && fixtures.iter().any(|f| f == server),
        None => false,
    }
}

/// The global options a principal's verb may not carry, **as the CLI
/// spells them** (the first cut named a field, `--tools`, and the real
/// flag `--tool` passed; found on review): everything that moves the
/// store, the model, the run's ceilings, the tool surface or the
/// approvals under the driver, in long and short form. Every `--no-*`
/// lever flag is refused by prefix. `mecha-cli` tests that each of these
/// is a global option it really defines.
pub const PRINCIPAL_BLOCKED_OPTIONS: [&str; 19] = [
    "--workspace",
    "-w",
    "--provider",
    "-p",
    "--model",
    "-m",
    "--effort",
    "-e",
    "--system",
    "-s",
    "--tool",
    "--skill",
    "--yes",
    "-y",
    "--read-only",
    "--max-turns",
    "--max-output-tokens",
    "--max-cost",
    "--compact-at",
];

/// The global options a principal's verb *may* carry: the ones that move
/// nothing about the run. `mecha-cli` walks every global option and fails
/// on one that is neither here, blocked, nor a `--no-` lever — the
/// direction a blocklist test has to face, or the next global option
/// lands unblocked (found on review).
pub const PRINCIPAL_HARMLESS_OPTIONS: [&str; 6] =
    ["--verbose", "-v", "--help", "-h", "--version", "-V"];

pub fn allowed_verb(verb: &[String]) -> bool {
    let head_allowed = PRINCIPAL_VERBS.iter().any(|lead| leads_with(verb, lead));
    let moves_the_run = verb.iter().any(|a| {
        let name = a.split('=').next().unwrap_or(a);
        // A short group: clap stacks flags (`-vw /tmp`) and lets the last
        // take the value, so every letter is an option — checking the
        // first alone read `-vw` as a harmless `-v` (found on review). Any
        // blocked letter anywhere in the group refuses the group; refusing
        // more than clap would parse narrows, never widens.
        if let Some(letters) = name.strip_prefix('-').filter(|s| !s.starts_with('-')) {
            return letters.chars().any(|c| {
                let opt = format!("-{c}");
                PRINCIPAL_BLOCKED_OPTIONS.contains(&opt.as_str())
            });
        }
        PRINCIPAL_BLOCKED_OPTIONS.contains(&name) || name.starts_with("--no-")
    });
    head_allowed && !moves_the_run
}

/// How often each loop stage runs between a lifetime's tasks: every N
/// tasks, 0 for never. Stages due after one position run in the nightly's
/// order (`scripts/ruminate.sh`) — reflect, **validate, then learn**,
/// ruminate — because `learn` marks reflections processed and `validate
/// --unprocessed-only` is the measurement that must see them first, or
/// the rules are graded on their own training data. The first cut ran
/// learn before validate and would have measured a loop that does not
/// ship (found on review).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    #[serde(default = "one")]
    pub reflect: u32,
    #[serde(default = "five")]
    pub learn: u32,
    #[serde(default = "five")]
    pub validate: u32,
    /// `rules propose-retirements --apply`: the nightly's one brake on
    /// rules that go live as they are derived. Runs after `learn`, as the
    /// nightly does; a loop that installed rules and never retired a
    /// harmful one would be more permissive than the one that ships, in
    /// the direction that flatters the `learn` arm (found on review).
    #[serde(default = "five")]
    pub retire: u32,
    #[serde(default = "ten")]
    pub ruminate: u32,
}

fn five() -> u32 {
    5
}
fn ten() -> u32 {
    10
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule {
            reflect: 1,
            learn: 5,
            validate: 5,
            retire: 5,
            ruminate: 10,
        }
    }
}

impl Schedule {
    /// The stages due after the task at `position` (1-based), in run order.
    pub fn due_after(&self, position: u32) -> Vec<StageLever> {
        let every = |n: u32| n > 0 && position.is_multiple_of(n);
        let mut out = Vec::new();
        if every(self.reflect) {
            out.push(StageLever::Reflect);
        }
        if every(self.validate) {
            out.push(StageLever::Validate);
        }
        if every(self.learn) {
            out.push(StageLever::Learn);
        }
        if every(self.retire) {
            out.push(StageLever::Retire);
        }
        if every(self.ruminate) {
            out.push(StageLever::Ruminate);
        }
        out
    }
}

/// The loop-stage levers, `lifetime` only (Part II §15's second table): a
/// closed set beside `harness::Lever`, each a stage the runner would run
/// between tasks and does not when the arm names it off. Four are verbs;
/// `sensors_in_brief` is `[agent] sensors_in_brief` in the trial home's
/// config — the homeostat's and guilt's entry into the diagnostician's
/// brief, which is the sensors' only reader. The stages the design names
/// and nothing has built (`followup_staging`, `prioritised_replay`) are not
/// here: a lever must be a switch that exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageLever {
    Reflect,
    Learn,
    Validate,
    Ruminate,
    /// `rules propose-retirements --apply`; see `Schedule::retire`.
    Retire,
    SensorsInBrief,
    /// Not a lever: the principal's call at a position, on the same ledger
    /// so `stage_health` and the judge's hold read it like a stage — a
    /// principal that failed to act is a treatment not known to have
    /// occurred. Not in `ALL`, never named in a manifest, runs no verb of
    /// its own.
    Principal,
    /// A stage this build cannot name, read off a ledger written by a
    /// build that has one — the closed-enum-on-an-append-only-store rule:
    /// the line reads as a stage run this build cannot name, never as a
    /// torn line. Not in `ALL`, never parsed from a manifest, runs nothing.
    #[serde(other)]
    Unknown,
}

impl StageLever {
    pub const ALL: [StageLever; 6] = [
        StageLever::Reflect,
        StageLever::Learn,
        StageLever::Validate,
        StageLever::Ruminate,
        StageLever::Retire,
        StageLever::SensorsInBrief,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            StageLever::Reflect => "reflect",
            StageLever::Learn => "learn",
            StageLever::Validate => "validate",
            StageLever::Ruminate => "ruminate",
            StageLever::Retire => "retire",
            StageLever::Principal => "principal",
            StageLever::SensorsInBrief => "sensors_in_brief",
            StageLever::Unknown => "unknown",
        }
    }

    pub fn parse(name: &str) -> Option<StageLever> {
        StageLever::ALL.into_iter().find(|l| l.as_str() == name)
    }

    pub fn names() -> String {
        StageLever::ALL
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The `mecha` verb a stage runs as, in the trial home — **the
    /// nightly's own argv** (`scripts/ruminate.sh`), so what a lifetime
    /// measures is the loop that ships: `validate --unprocessed-only` is
    /// the held-out measurement, `learn --holdout 0.25 --auto` keeps a
    /// quarter of reflections unseen for the next one. `None` for the
    /// lever that is a config switch rather than a stage.
    pub fn argv(self) -> Option<&'static [&'static str]> {
        match self {
            StageLever::Reflect => Some(&["reflect"]),
            StageLever::Learn => Some(&["learn", "--holdout", "0.25", "--auto"]),
            StageLever::Validate => Some(&["validate", "--unprocessed-only"]),
            StageLever::Ruminate => Some(&["harness", "ruminate"]),
            StageLever::Retire => Some(&["rules", "propose-retirements", "--apply"]),
            StageLever::SensorsInBrief | StageLever::Principal | StageLever::Unknown => None,
        }
    }
}

fn one() -> u32 {
    1
}
fn three() -> u64 {
    3
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialKind {
    #[default]
    Single,
    Lifetime,
}

/// One arm: which levers are off, which knobs are moved, and — for a
/// treatment arm — what it predicts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arm {
    /// A named starting point, applied before `levers_off`. `bare` is what
    /// `mecha eval` runs (`Lever::bare`, every lever off but the operator's
    /// rules); `full` is every lever on. Part II §15: "appraisal off" is a
    /// preset over consumer levers, never a lever, and the manifest names
    /// presets as such so a reader a month later can tell what was absent.
    #[serde(default)]
    pub preset: Option<Preset>,
    #[serde(default)]
    pub levers_off: Vec<String>,
    /// Levers turned back *on* after the preset — the add-one-to-bare
    /// design (Part II §15: "only for a lever with a prior worth testing in
    /// isolation"), and how `mecha eval --ab-rules` is spelled as an arm:
    /// `bare` plus `learned_rules`. Applied after `levers_off`, so a name in
    /// both is on; the `approval_rules` refusal applies here too.
    #[serde(default)]
    pub levers_on: Vec<String>,
    /// `KEY=VALUE` over `harness::OverrideKey`, validated at load.
    #[serde(default)]
    pub overrides: Vec<String>,
    /// The provider this arm runs against, by the operator's config key;
    /// the operator's default when absent. The **other axis** of a
    /// condition: an arm is a model and a harness configuration, and an
    /// experiment may vary either or both — eval's model bake-off is the
    /// special case of arms that name models under the `bare` preset.
    #[serde(default)]
    pub provider: Option<String>,
    /// The model id, overriding the provider's default.
    #[serde(default)]
    pub model: Option<String>,
    /// Loop stages this arm does *not* run between tasks, by
    /// [`StageLever`] name — a `lifetime` manifest only; a `single` one
    /// refuses them at load, since no stage ever runs there and a lever
    /// that changes nothing would still move the hash.
    #[serde(default)]
    pub stages_off: Vec<String>,
    #[serde(default)]
    pub prediction: Option<Prediction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Bare,
    Full,
}

/// What a treatment arm predicts, made before either arm is measured.
/// `candidate::Prediction`'s shape with one more metric: the task outcome,
/// entering as a cost (D4 — every metric is lower-is-better, so a solve
/// rate enters as `1 − solved` and never as a benefit axis).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prediction {
    pub metric: ExpMetric,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpMetric {
    /// `1.0` for a trial whose grader failed it, `0.0` for a pass — the
    /// task outcome as a cost.
    Failure,
    /// `candidate::Metric`'s six, over the trial's folded `RunStats`.
    EndedOnFailedCall,
    ToolErrorRate,
    CutShort,
    Compactions,
    Turns,
    MalformedArgs,
}

impl ExpMetric {
    fn stats_metric(self) -> Option<Metric> {
        Some(match self {
            ExpMetric::Failure => return None,
            ExpMetric::EndedOnFailedCall => Metric::EndedOnFailedCall,
            ExpMetric::ToolErrorRate => Metric::ToolErrorRate,
            ExpMetric::CutShort => Metric::CutShort,
            ExpMetric::Compactions => Metric::Compactions,
            ExpMetric::Turns => Metric::Turns,
            ExpMetric::MalformedArgs => Metric::MalformedArgs,
        })
    }

    /// The cost of one finished trial under this metric. `None` when the
    /// trial cannot answer — no grade for `failure`, no stats for the rest —
    /// which drops the pair rather than scoring an unknown as zero.
    pub fn cost(self, trial: &Trial) -> Option<f64> {
        match self.stats_metric() {
            None => trial.passed.map(|p| if p { 0.0 } else { 1.0 }),
            Some(m) => trial.stats.as_ref().map(|s| m.of(s)),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ExpMetric::Failure => "failure",
            ExpMetric::EndedOnFailedCall => "ended_on_failed_call",
            ExpMetric::ToolErrorRate => "tool_error_rate",
            ExpMetric::CutShort => "cut_short",
            ExpMetric::Compactions => "compactions",
            ExpMetric::Turns => "turns",
            ExpMetric::MalformedArgs => "malformed_args",
        }
    }
}

/// Where the tasks come from: an eval case file, as `mecha eval` reads it,
/// and the fixture workspace its cases run against.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tasks {
    pub cases: PathBuf,
    pub fixture: PathBuf,
    /// Only these case ids, when set.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Only cases carrying one of these tags, when set.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Manifest {
    /// A two-arm design built by a front-end rather than written by hand —
    /// how `mecha eval`'s `--ab-config` and `--ab-rules` are spelled as
    /// experiments: the control is `bare` (what eval runs), the treatment
    /// is the control plus one delta, predicting a lower failure cost. The
    /// split seed is derived from the treatment's own description, so a
    /// rerun of the same A/B *over the same case set* holds out the same
    /// tasks — the draw is over pair indices, so a case added to the file
    /// reshuffles it, where the hash-by-id holdout eval used kept per-case
    /// membership fixed; the store's mechanism, with that narrower promise
    /// — and nothing else about the design is chosen after a trial ran.
    /// Validated like a parsed one —
    /// `treatment_name` is a directory name, so the delta itself goes in
    /// the prediction's rationale, not the name.
    #[allow(clippy::too_many_arguments)]
    pub fn two_arm(
        name: &str,
        treatment_name: &str,
        mut treatment: Arm,
        tasks: Tasks,
        holdout_in: u64,
        repetitions: u32,
        shared_levers_on: &[String],
        shared_overrides: &[String],
    ) -> Result<Manifest> {
        // What both arms carry over the preset: the levers eval opts back
        // in (`--mcp`), and the knobs both arms inherit from the machine and
        // the flags (`max_turns`, `compact_at_tokens`, `max_output_tokens`,
        // `effort`) — so the record says what the control ran rather than
        // `bare` for a control that had its MCP servers or a `--max-turns
        // 60` nobody wrote down; the condition hash follows (found on
        // review, both halves). The treatment's own overrides come last and
        // win.
        let mut arms = BTreeMap::new();
        arms.insert(
            "bare".to_string(),
            Arm {
                preset: Some(Preset::Bare),
                levers_on: shared_levers_on.to_vec(),
                overrides: shared_overrides.to_vec(),
                ..Arm::default()
            },
        );
        let own = std::mem::take(&mut treatment.overrides);
        // Seeded from the delta alone — the treatment's own overrides and
        // levers before anything shared is merged in — so the promise is per
        // delta and case set, not per machine configuration or opt-in (found
        // on review, twice: the knobs, then the levers).
        let split_seed = {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in format!(
                "{}|{:?}|{:?}|{:?}",
                treatment_name, treatment.levers_off, treatment.levers_on, own
            )
            .bytes()
            {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            // A TOML integer is 64-bit *signed*, and the manifest round-trips
            // through TOML — so the seed lives in the bottom half of the
            // range or it cannot be written down (found on review).
            h & (u64::MAX >> 1)
        };
        for lever in shared_levers_on {
            if !treatment.levers_on.contains(lever) {
                treatment.levers_on.push(lever.clone());
            }
        }
        let key_of = |s: &str| s.split_once('=').map(|(k, _)| k.trim().to_string());
        let mut merged: Vec<String> = shared_overrides
            .iter()
            .filter(|s| !own.iter().any(|o| key_of(o) == key_of(s)))
            .cloned()
            .collect();
        merged.extend(own);
        treatment.overrides = merged;
        arms.insert(treatment_name.to_string(), treatment);
        let m = Manifest {
            name: name.to_string(),
            description: String::new(),
            kind: TrialKind::Single,
            control: Some("bare".to_string()),
            arms,
            tasks,
            seeds: Vec::new(),
            repetitions,
            split_seed,
            holdout_in,
            schedule: Schedule::default(),
            principal: None,
            fixtures: Fixtures::default(),
        };
        m.validate()?;
        Ok(m)
    }

    /// A measurement built by a front-end: one arm, no control, no
    /// prediction — what a plain `mecha eval` is, recorded so a scorecard's
    /// condition (model, preset, the machine's knobs) is on the store beside
    /// every comparison's. The split seed is fixed at zero: nothing is
    /// drawn from a measurement.
    pub fn one_arm(
        name: &str,
        arm_name: &str,
        arm: Arm,
        tasks: Tasks,
        repetitions: u32,
    ) -> Result<Manifest> {
        anyhow::ensure!(
            arm.prediction.is_none(),
            "a measurement's arm predicts nothing: there is no control to measure it against"
        );
        let mut arms = BTreeMap::new();
        arms.insert(arm_name.to_string(), arm);
        let m = Manifest {
            name: name.to_string(),
            description: String::new(),
            kind: TrialKind::Single,
            control: None,
            arms,
            tasks,
            seeds: Vec::new(),
            repetitions,
            split_seed: 0,
            holdout_in: 3,
            schedule: Schedule::default(),
            principal: None,
            fixtures: Fixtures::default(),
        };
        m.validate()?;
        Ok(m)
    }

    /// Parse and validate. Every rule the module doc names is enforced here
    /// and nowhere else, so a `Manifest` value is one that passed.
    pub fn parse(text: &str) -> Result<Manifest> {
        let m: Manifest = toml::from_str(text).context("parsing the manifest")?;
        m.validate()?;
        Ok(m)
    }

    pub fn load(path: &Path) -> Result<Manifest> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Manifest::parse(&text).with_context(|| path.display().to_string())
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(!self.name.is_empty(), "the manifest needs a name");
        crate::work::valid_producer(&self.name)
            .context("the experiment name is a directory name and a producer name")?;
        anyhow::ensure!(
            !self.arms.is_empty(),
            "an experiment needs at least one arm"
        );
        if let Some(control) = &self.control {
            anyhow::ensure!(
                self.arms.contains_key(control),
                "control `{control}` names no arm (arms: {})",
                self.arms.keys().cloned().collect::<Vec<_>>().join(", ")
            );
            anyhow::ensure!(
                self.arms.len() >= 2,
                "a comparison needs the control and at least one treatment arm; leave `control` unset for a measurement"
            );
        }
        anyhow::ensure!(self.repetitions >= 1, "repetitions must be at least 1");
        anyhow::ensure!(self.holdout_in >= 2, "holdout_in must be at least 2");
        self.fixtures.validate()?;
        // Both kinds: `ids` names each case once. For a lifetime the
        // positions are the tasks; for a single, `cases_for` walks `ids` in
        // order and a repeated id would plan two rows with one trial id —
        // one overwriting the other on the store (found on review).
        let mut seen = std::collections::BTreeSet::new();
        for id in &self.tasks.ids {
            anyhow::ensure!(
                seen.insert(id),
                "task `{id}` appears twice in `ids`; name each task once"
            );
        }
        // The same collision one field over: a seed twice mints two rows
        // with one trial id, and two lifetimes in one home and one ledger
        // (found on review).
        let mut seen_seeds = std::collections::BTreeSet::new();
        for s in &self.seeds {
            anyhow::ensure!(
                seen_seeds.insert(s),
                "seed `{s}` appears twice in `seeds`; name each seed once"
            );
        }
        match self.kind {
            TrialKind::Single => {
                anyhow::ensure!(
                    self.schedule == Schedule::default(),
                    "a `[schedule]` is a lifetime's; a single trial runs no stage between tasks"
                );
                anyhow::ensure!(
                    self.principal.is_none(),
                    "a `[principal]` is a lifetime's; a single trial has no position for one to act at"
                );
            }
            // The sequence is the design, and the case file is not: a
            // lifetime that fell back to the file's order kept each
            // finished row's stored position while a changed file moved
            // the walk under it, and nothing on the record said so (found
            // on review).
            TrialKind::Lifetime => {
                anyhow::ensure!(
                    !self.tasks.ids.is_empty(),
                    "a `lifetime` names its sequence in `[tasks] ids`; the case file's order is not a design"
                );
                if let Some(p) = &self.principal {
                    anyhow::ensure!(
                        !p.command.is_empty(),
                        "`[principal] command` names no executable"
                    );
                    anyhow::ensure!(
                        p.timeout_secs > 0,
                        "`[principal] timeout_secs` must be positive"
                    );
                }
            }
        }
        for (name, arm) in &self.arms {
            crate::work::valid_producer(name)
                .with_context(|| format!("arm `{name}` is a directory name"))?;
            arm.resolve_levers()
                .with_context(|| format!("arm `{name}`"))?;
            let stages = arm
                .resolve_stages()
                .with_context(|| format!("arm `{name}`"))?;
            anyhow::ensure!(
                self.kind == TrialKind::Lifetime || stages.is_empty(),
                "arm `{name}` names stage lever(s) off, but this is a `single` experiment and no stage runs between its trials"
            );
            for spec in &arm.overrides {
                crate::harness::parse_change(spec)
                    .with_context(|| format!("arm `{name}`, override `{spec}`"))?;
            }
            match &self.control {
                None => anyhow::ensure!(
                    arm.prediction.is_none(),
                    "arm `{name}` carries a prediction but the manifest names no control to measure it against; set `control`, or drop the prediction for a measurement"
                ),
                Some(c) if c == name => anyhow::ensure!(
                    arm.prediction.is_none(),
                    "the control arm `{name}` must not carry a prediction — it is what the others are measured against"
                ),
                Some(_) => anyhow::ensure!(
                    arm.prediction.is_some(),
                    "treatment arm `{name}` carries no prediction; an arm that predicts nothing cannot be refuted (candidate.rs's rule, carried up a level)"
                ),
            }
        }
        Ok(())
    }

    /// Every trial the design calls for, in a stable order, each with its
    /// condition hash. `provider` and `model` are the operator's defaults;
    /// an arm that names its own overrides them, and the hash follows the
    /// arm. Pure: the store decides which have run.
    pub fn trials(&self, task_ids: &[String], provider: &str, model: &str) -> Vec<Trial> {
        let seeds: Vec<Option<u64>> = if self.seeds.is_empty() {
            vec![None]
        } else {
            self.seeds.iter().copied().map(Some).collect()
        };
        let mut out = Vec::new();
        let fixtures = self.fixtures.names();
        for (arm_name, arm) in &self.arms {
            let resolved = arm.resolve_levers().expect("validated at load");
            let stages = arm.resolve_stages().expect("validated at load");
            let provider = arm.provider.as_deref().unwrap_or(provider);
            let model = arm.model.as_deref().unwrap_or(model);
            let row = |task: &String, seed: Option<u64>, rep: u32, position: Option<u32>| Trial {
                id: trial_id(arm_name, task, seed, rep),
                arm: arm_name.clone(),
                task: task.clone(),
                seed,
                repetition: rep,
                condition_hash: condition_hash_of(
                    &resolved,
                    &arm.overrides,
                    provider,
                    model,
                    seed,
                    &stages,
                    &fixtures,
                ),
                status: TrialStatus::Pending,
                session_id: None,
                started_at: None,
                finished_at: None,
                error: None,
                passed: None,
                checks: Vec::new(),
                stats: None,
                position,
                lifetime: position.map(|_| lifetime_id(arm_name, seed, rep)),
            };
            match self.kind {
                TrialKind::Single => {
                    for task in task_ids {
                        for seed in &seeds {
                            for rep in 1..=self.repetitions {
                                out.push(row(task, *seed, rep, None));
                            }
                        }
                    }
                }
                // One home per (arm × seed × repetition), the sequence in
                // order inside it: a lifetime's rows are contiguous and
                // positioned, so a driver walks them as written.
                TrialKind::Lifetime => {
                    for seed in &seeds {
                        for rep in 1..=self.repetitions {
                            for (i, task) in task_ids.iter().enumerate() {
                                out.push(row(task, *seed, rep, Some(i as u32 + 1)));
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

/// The home id of one lifetime: an arm, a seed, a repetition.
pub fn lifetime_id(arm: &str, seed: Option<u64>, rep: u32) -> String {
    match seed {
        Some(s) => format!("{arm}__s{s}__r{rep}"),
        None => format!("{arm}__r{rep}"),
    }
}

impl Arm {
    /// The levers this arm carries off, in `Lever::ALL`'s order: the preset
    /// first, then `levers_off` by name. Errors are the load-time rules.
    pub fn resolve_levers(&self) -> Result<Vec<Lever>> {
        let mut off: Vec<Lever> = match self.preset {
            Some(Preset::Bare) => Lever::bare(&[]),
            Some(Preset::Full) | None => Vec::new(),
        };
        let parse = |name: &str| -> Result<Lever> {
            let lever = Lever::parse(name).with_context(|| {
                format!(
                    "`{name}` is not a lever (the closed set: {})",
                    Lever::names()
                )
            })?;
            anyhow::ensure!(
                lever != Lever::ApprovalRules,
                "`approval_rules` cannot be a lever in an experiment: a forbid is the operator's standing word, and only mecha eval's fixture workspaces justify lifting it"
            );
            Ok(lever)
        };
        for name in &self.levers_off {
            off.push(parse(name)?);
        }
        let mut on = Vec::new();
        for name in &self.levers_on {
            on.push(parse(name)?);
        }
        Ok(Lever::ALL
            .into_iter()
            .filter(|l| off.contains(l) && !on.contains(l))
            .collect())
    }

    /// The stage levers this arm carries off, in `StageLever::ALL`'s order.
    /// An unknown name is the load error, never a skipped line.
    pub fn resolve_stages(&self) -> Result<Vec<StageLever>> {
        let mut off = Vec::new();
        for name in &self.stages_off {
            let lever = StageLever::parse(name).with_context(|| {
                format!(
                    "`{name}` is not a stage lever (the closed set: {})",
                    StageLever::names()
                )
            })?;
            off.push(lever);
        }
        Ok(StageLever::ALL
            .into_iter()
            .filter(|l| off.contains(l))
            .collect())
    }
}

fn trial_id(arm: &str, task: &str, seed: Option<u64>, rep: u32) -> String {
    let task: String = task
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    match seed {
        Some(s) => format!("{arm}__{task}__s{s}__r{rep}"),
        None => format!("{arm}__{task}__r{rep}"),
    }
}

/// Hash of the resolved arm — levers off, overrides, provider, model, seed.
/// Two runs with the same hash were configured identically; two with
/// different hashes differ somewhere, even if nobody remembers where (§4).
/// FNV-1a over a canonical rendering: an equality key, not a credential, so
/// no hashing dependency is worth adding for it.
pub fn condition_hash(
    levers_off: &[Lever],
    overrides: &[String],
    provider: &str,
    model: &str,
    seed: Option<u64>,
) -> String {
    condition_hash_with_stages(levers_off, overrides, provider, model, seed, &[])
}

/// [`condition_hash`] for a lifetime's row: the stage levers off are part
/// of the condition a row ran under, since a stage between tasks changes
/// what the next task starts from. The `stages_off=` term is appended
/// only when a stage is off, so every hash minted before stage levers
/// existed — every `single` row and every eval's — keeps its value, and a
/// lifetime arm with every stage on hashes as its single-trial twin.
pub fn condition_hash_with_stages(
    levers_off: &[Lever],
    overrides: &[String],
    provider: &str,
    model: &str,
    seed: Option<u64>,
    stages_off: &[StageLever],
) -> String {
    condition_hash_of(
        levers_off,
        overrides,
        provider,
        model,
        seed,
        stages_off,
        &[],
    )
}

/// [`condition_hash_with_stages`] with the fixture servers a row ran
/// against: a trial whose home carried a fixture board and mailbox ran on
/// a different tool surface from one on the operator's, and two such rows
/// must not pair. The `fixtures=` term is appended only when a manifest
/// names any, so every hash minted before fixtures existed keeps its value.
pub fn condition_hash_of(
    levers_off: &[Lever],
    overrides: &[String],
    provider: &str,
    model: &str,
    seed: Option<u64>,
    stages_off: &[StageLever],
    fixtures: &[String],
) -> String {
    let mut overrides: Vec<&str> = overrides.iter().map(String::as_str).collect();
    overrides.sort_unstable();
    let mut canonical = format!(
        "levers_off={}|overrides={}|provider={provider}|model={model}|seed={}",
        levers_off
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(","),
        overrides.join(","),
        seed.map(|s| s.to_string()).unwrap_or_default()
    );
    if !stages_off.is_empty() {
        canonical.push_str("|stages_off=");
        canonical.push_str(
            &stages_off
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    if !fixtures.is_empty() {
        let mut names: Vec<&str> = fixtures.iter().map(String::as_str).collect();
        names.sort_unstable();
        canonical.push_str("|fixtures=");
        canonical.push_str(&names.join(","));
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in canonical.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

// ─── Trials ─────────────────────────────────────────────────────────────────

/// One row per arm × task × seed × repetition. Written by the runner as the
/// trial moves; read by `status` and `judge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trial {
    pub id: String,
    pub arm: String,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    pub repetition: u32,
    pub condition_hash: String,
    #[serde(default, deserialize_with = "de_lenient_status")]
    pub status: TrialStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The grader's verdict. `None` until graded — never `false`, because
    /// "not yet graded" and "failed" are opposite findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<crate::eval::Check>,
    /// The child run's folded outcome, read off its session in the trial home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<RunStats>,
    /// A lifetime's rows only: this task's 1-based place in the sequence,
    /// and the home id (`lifetime_id`) the row ran in. `None` on a single
    /// trial's row and on every row written before lifetimes existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<String>,
}

impl Trial {
    /// A trial that finished in-process — how `mecha eval` files each arm's
    /// case result as a row: graded, with the run's folded stats, and no
    /// session (eval writes none).
    pub fn finished(
        planned: &Trial,
        passed: bool,
        checks: Vec<crate::eval::Check>,
        stats: Option<RunStats>,
    ) -> Trial {
        let now = chrono::Utc::now().to_rfc3339();
        Trial {
            status: TrialStatus::Done,
            started_at: Some(now.clone()),
            finished_at: Some(now),
            passed: Some(passed),
            checks,
            stats,
            ..planned.clone()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialStatus {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
    /// A status this build does not know. A trial in this state is neither
    /// rerun nor judged.
    Unknown,
}

fn de_lenient_status<'de, D>(d: D) -> std::result::Result<TrialStatus, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or(TrialStatus::Unknown))
}

// ─── The store ──────────────────────────────────────────────────────────────

/// `~/.mecha/experiments/<name>/` — the manifest, the trials, and the
/// isolated homes the trials run in.
pub struct ExperimentStore {
    root: PathBuf,
}

/// The marker the runner writes into a trial home. `runlog::Scan` admits
/// `SessionKind::Experiment` by default only in a home carrying it (D13):
/// an experiment session that leaked into the real store stays hidden where
/// it would contaminate, and is read where it belongs.
pub const HOME_MARKER: &str = "EXPERIMENT";

impl ExperimentStore {
    pub fn root_default() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("experiments"))
    }

    pub fn open(root: impl Into<PathBuf>, name: &str) -> Result<ExperimentStore> {
        crate::work::valid_producer(name)?;
        Ok(ExperimentStore {
            root: root.into().join(name),
        })
    }

    pub fn open_default(name: &str) -> Result<ExperimentStore> {
        ExperimentStore::open(Self::root_default()?, name)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.toml")
    }

    /// Create the experiment from a manifest. Refuses to overwrite: a design
    /// is written once, before the run, and a second `new` over it would be
    /// the after-the-fact redesign the manifest exists to prevent.
    pub fn create(&self, manifest_text: &str) -> Result<Manifest> {
        let manifest = Manifest::parse(manifest_text)?;
        anyhow::ensure!(
            !self.manifest_path().exists(),
            "{} already exists; an experiment's design is written once",
            self.manifest_path().display()
        );
        std::fs::create_dir_all(self.root.join("trials"))?;
        write_atomic(&self.manifest_path(), manifest_text.as_bytes())?;
        Ok(manifest)
    }

    /// [`Self::create`] for a design built in code (`Manifest::two_arm`).
    pub fn create_manifest(&self, manifest: &Manifest) -> Result<()> {
        let text = toml::to_string_pretty(manifest).context("rendering the manifest")?;
        self.create(&text).map(|_| ())
    }

    pub fn manifest(&self) -> Result<Manifest> {
        Manifest::load(&self.manifest_path())
    }

    pub fn trial_path(&self, id: &str) -> PathBuf {
        self.root.join("trials").join(format!("{id}.json"))
    }

    pub fn save_trial(&self, t: &Trial) -> Result<()> {
        std::fs::create_dir_all(self.root.join("trials"))?;
        write_atomic(
            &self.trial_path(&t.id),
            serde_json::to_string_pretty(t)?.as_bytes(),
        )
    }

    /// Every trial on disk, by id. A file that does not parse is a finding
    /// (`skipped`), not an empty row.
    pub fn trials(&self) -> Result<(BTreeMap<String, Trial>, usize)> {
        let dir = self.root.join("trials");
        let mut out = BTreeMap::new();
        let mut skipped = 0;
        if !dir.exists() {
            return Ok((out, 0));
        }
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<Trial>(&s).ok())
            {
                Some(t) => {
                    out.insert(t.id.clone(), t);
                }
                None => skipped += 1,
            }
        }
        Ok((out, skipped))
    }

    /// The design's trials merged with what is on disk: a trial the store
    /// knows keeps its row; one it does not is pending. The store's rows
    /// never lose to the design — a finished trial stays finished.
    pub fn plan(
        &self,
        manifest: &Manifest,
        task_ids: &[String],
        provider: &str,
        model: &str,
    ) -> Result<(Vec<Trial>, usize)> {
        let (on_disk, skipped) = self.trials()?;
        let planned = manifest
            .trials(task_ids, provider, model)
            .into_iter()
            .map(|t| on_disk.get(&t.id).cloned().unwrap_or(t))
            .collect();
        Ok((planned, skipped))
    }

    /// The isolated home one arm's trials run in (D12). Created with the
    /// marker, and **refused if it is, or contains, the real home**.
    pub fn arm_home(&self, arm: &str) -> Result<PathBuf> {
        self.home_at(arm)
    }

    /// The isolated home one *lifetime* runs in — one per arm × seed ×
    /// repetition (`lifetime_id`), because a lifetime's whole point is what
    /// its stages leave in the store for the next task, and two lifetimes
    /// sharing a home would learn from each other.
    pub fn lifetime_home(&self, lifetime: &str) -> Result<PathBuf> {
        self.home_at(lifetime)
    }

    fn home_at(&self, name: &str) -> Result<PathBuf> {
        let home = self.root.join("homes").join(name);
        let real = crate::work::mecha_home()?;
        refuse_unsafe_home(&home, &real)?;
        let fresh = !home.join(HOME_MARKER).exists();
        std::fs::create_dir_all(&home)?;
        if fresh {
            seed_home(&real, &home)?;
        }
        std::fs::write(
            home.join(HOME_MARKER),
            b"an experiment home; see mecha exp\n",
        )?;
        Ok(home)
    }

    pub fn workspace_for(&self, trial_id: &str) -> PathBuf {
        self.root.join("trials").join(trial_id).join("workspace")
    }

    /// One lifetime's stage ledger: `stages/<lifetime>.jsonl`, appended per
    /// stage run. The ledger is what says a stage ran — the manifest says
    /// only what was scheduled — and what a resumed driver reads to run a
    /// stage the crash fell between.
    pub fn stage_ledger(&self, lifetime: &str) -> PathBuf {
        self.root.join("stages").join(format!("{lifetime}.jsonl"))
    }

    /// The directory a stage runs *from*. Not the trial home: a verb that
    /// builds a path jail from its cwd (`validate`'s probes replay tool
    /// calls) refuses a workspace that contains the mecha home, and the
    /// home is exactly what a stage's cwd would have been (found on the
    /// first live lifetime). An empty directory beside the ledger — no
    /// `mecha.toml` can layer over the arm from there either.
    pub fn stage_workspace(&self, lifetime: &str) -> PathBuf {
        self.root.join("stages").join(lifetime).join("workspace")
    }

    /// Where a stage's output lands. Keyed by attempt as well: the ledger
    /// keeps a failed stage's line and the rerun's as two lines, so the
    /// logs must be two files, or the failed line points at the rerun's
    /// output (found on review).
    pub fn stage_log(
        &self,
        lifetime: &str,
        after_position: u32,
        stage: StageLever,
        attempt: u32,
    ) -> PathBuf {
        self.root.join("stages").join(lifetime).join(format!(
            "{after_position:03}-{}-a{attempt}.log",
            stage.as_str()
        ))
    }

    /// The attempt number the next run of `stage` after `position` gets:
    /// one more than the ledger holds for that pair, **plus every torn
    /// line** — a line that did not parse may have been this pair's, and
    /// an attempt number reused would truncate the log the suffix exists
    /// to keep (found on review). Monotonic, not dense: a torn line on
    /// another pair skips a number here, which costs nothing.
    pub fn next_attempt(ledger: &[StageRun], torn: usize, position: u32, stage: StageLever) -> u32 {
        ledger
            .iter()
            .filter(|r| r.after_position == position && r.stage == stage)
            .count() as u32
            + torn as u32
            + 1
    }

    pub fn record_stage(&self, run: &StageRun) -> Result<()> {
        use std::io::Write;
        let path = self.stage_ledger(&run.lifetime);
        std::fs::create_dir_all(path.parent().expect("under the root"))?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        let mut line = serde_json::to_string(run)?;
        line.push('\n');
        f.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Every lifetime's stage runs on this store, and the torn-line count
    /// across them — what `judge` and `export` read, since for a lifetime
    /// the ledger is the evidence that a treatment occurred.
    pub fn all_stage_runs(&self) -> Result<(Vec<StageRun>, usize)> {
        let dir = self.root.join("stages");
        let (mut out, mut torn) = (Vec::new(), 0);
        if !dir.exists() {
            return Ok((out, torn));
        }
        let mut names: Vec<String> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.strip_suffix(".jsonl").map(str::to_string)
            })
            .collect();
        names.sort();
        for lifetime in names {
            let (runs, t) = self.stage_runs(&lifetime)?;
            out.extend(runs);
            torn += t;
        }
        Ok((out, torn))
    }

    /// The stage runs on one lifetime's ledger, in order, and the count of
    /// lines that did not parse — a torn line is a finding, not a stage
    /// that never ran.
    pub fn stage_runs(&self, lifetime: &str) -> Result<(Vec<StageRun>, usize)> {
        let path = self.stage_ledger(lifetime);
        if !path.exists() {
            return Ok((Vec::new(), 0));
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut out = Vec::new();
        let mut skipped = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<StageRun>(line) {
                Ok(r) => out.push(r),
                Err(_) => skipped += 1,
            }
        }
        Ok((out, skipped))
    }
}

/// One stage run between a lifetime's tasks, on the ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageRun {
    pub lifetime: String,
    pub arm: String,
    pub stage: StageLever,
    /// The position whose task ran just before this stage.
    pub after_position: u32,
    /// Which attempt this is for the (position, stage) pair; the running
    /// line and its terminal line share one. Rows written before the
    /// field existed read as attempt 1.
    #[serde(default = "one")]
    pub attempt: u32,
    pub started_at: String,
    pub finished_at: String,
    pub status: StageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A principal line only: which point it was called at, and the verbs
    /// it asked for, each with how the driver's run of it went.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<PrincipalPoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acts: Vec<PrincipalAct>,
    /// An `after_task` principal line only: each refusal scripted for the
    /// task, with how many times the run answered a call with it — read
    /// off the task's session, where the loop writes the refusal's exact
    /// sentence. A refusal that never fired is a treatment that did not
    /// occur at this position, and without this it was recorded like one
    /// that did (found on review).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refusals: Vec<RefusalOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefusalOutcome {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_contains: Option<String>,
    pub reason: String,
    /// How many times the run answered a call with this refusal; `None`
    /// when the session could not be read — unknown, never zero (found on
    /// review).
    #[serde(default)]
    pub fired: Option<u32>,
}

/// The sentence the loop writes for a person's refusal, as `agent.rs`
/// renders `Decision::Deny`.
pub const DENIED_PREFIX: &str = "Denied by the user: ";

/// How often each scripted refusal fired in a session: the count of the
/// loop's refusal sentence carrying that rule's reason, over the session
/// file's text — or unknown for every rule when there is no text to read.
/// Two rules sharing a reason share a count, and saying so is the
/// principal's job.
pub fn refusal_outcomes(
    rules: &[crate::tool::DenialRule],
    session_text: Option<&str>,
) -> Vec<RefusalOutcome> {
    rules
        .iter()
        .map(|r| RefusalOutcome {
            tool: r.tool.clone(),
            input_contains: r.input_contains.clone(),
            reason: r.reason.clone(),
            // Encoded the way the file holds it: the sentence sits in a
            // JSON string, so a quote, a backslash or a newline in the
            // reason is escaped there and a raw needle never matched
            // (found on review).
            fired: session_text.map(|text| {
                let sentence = format!("{DENIED_PREFIX}{}", r.reason);
                let encoded = serde_json::to_string(&sentence).unwrap_or_default();
                // The encoding's own quotes, and only those: a reason that
                // ends in a quote must keep it.
                let needle = &encoded[1..encoded.len().saturating_sub(1).max(1)];
                text.matches(needle).count() as u32
            }),
        })
        .collect()
}

/// The refusals the home's denials file holds, for the driver to read
/// back after the task; none when there is no file.
pub fn read_denials(home: &Path) -> Result<Vec<crate::tool::DenialRule>> {
    let path = denials_file(home);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: crate::tool::DenialsFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(file.rules)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    /// Appended *before* the child is spawned, so a driver killed
    /// mid-stage leaves a line: the attempt number is burnt and the rerun
    /// cannot truncate the interrupted attempt's log. A `running` line no
    /// terminal line supersedes (same position, stage and attempt) reads
    /// as an interrupted stage (found on review).
    Running,
    Done,
    Failed,
    /// Due, but a later position had already finished when the driver came
    /// back to it: a stage that ran now would act on sessions its tasks
    /// never ran under — causally inert, and a success that would have
    /// released the judge's hold on a treatment that did not occur (found
    /// on review). Terminal: never rerun, counted as broken, and it
    /// supersedes the failure it stands for so the count is not doubled.
    Skipped,
    /// A status this build does not know: counted neither done nor failed,
    /// and **due again** (`stages_due` reruns anything not `done`), because
    /// a status this build cannot read is not proof the stage finished.
    #[serde(other)]
    Unknown,
}

/// How a set of stage runs went — one lifetime's, or every lifetime of an
/// arm's. `interrupted` is a `running` line no terminal line supersedes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct StageHealth {
    pub done: usize,
    pub failed: usize,
    pub interrupted: usize,
    /// Due stages the driver could no longer run in sequence.
    pub skipped: usize,
    pub unknown: usize,
}

impl StageHealth {
    /// A stage not known to have run as designed: failed, interrupted, or
    /// in a status this build cannot read — unknown is never clean. A
    /// failed or unknown line a later `done` line supersedes for the same
    /// stage is not counted: `stages_due` reruns a failed stage, and the
    /// rerun's success is the ledger saying it ran (found on review —
    /// without this, one transient failure held every verdict at propose
    /// forever).
    pub fn broken(&self) -> usize {
        self.failed + self.interrupted + self.skipped + self.unknown
    }
}

pub fn stage_health(runs: &[StageRun]) -> StageHealth {
    let mut h = StageHealth::default();
    // A later attempt that settled the pair — ran, or was recorded skipped
    // out of sequence — stands for every earlier line of it.
    // A line's identity is its lifetime, position, stage *and point*: the
    // two principal calls at one position share the first three, and
    // `after_task` always takes the higher attempt, so without the point a
    // failed `before_task` was stood for by the `after_task`'s success
    // and the hold never fired (found on review).
    let same_pair = |t: &StageRun, r: &StageRun| {
        t.lifetime == r.lifetime
            && t.after_position == r.after_position
            && t.stage == r.stage
            && t.point == r.point
    };
    let later_settled = |r: &StageRun| {
        runs.iter().any(|t| {
            matches!(t.status, StageStatus::Done | StageStatus::Skipped)
                && same_pair(t, r)
                && t.attempt > r.attempt
        })
    };
    for r in runs {
        match r.status {
            StageStatus::Done => h.done += 1,
            StageStatus::Skipped => h.skipped += 1,
            StageStatus::Failed => {
                if !later_settled(r) {
                    h.failed += 1;
                }
            }
            StageStatus::Unknown => {
                if !later_settled(r) {
                    h.unknown += 1;
                }
            }
            StageStatus::Running => {
                // Its own terminal line, or a later attempt's success: the
                // rerun always takes a higher attempt number, so without
                // the second clause one interrupt pinned every verdict at
                // propose forever (found on review).
                let superseded = runs.iter().any(|t| {
                    t.status != StageStatus::Running && same_pair(t, r) && t.attempt == r.attempt
                }) || later_settled(r);
                if !superseded {
                    h.interrupted += 1;
                }
            }
        }
    }
    h
}

/// The stages still due after `position`: the schedule's, minus the arm's
/// levers off, minus those the ledger already shows done after that
/// position. A failed stage is due again — the ledger keeps the failure,
/// and the rerun is a second line, never an overwrite.
pub fn stages_due(
    schedule: &Schedule,
    position: u32,
    stages_off: &[StageLever],
    ledger: &[StageRun],
) -> Vec<StageLever> {
    schedule
        .due_after(position)
        .into_iter()
        .filter(|s| !stages_off.contains(s))
        .filter(|s| {
            !ledger.iter().any(|r| {
                r.after_position == position
                    && r.stage == *s
                    && matches!(r.status, StageStatus::Done | StageStatus::Skipped)
            })
        })
        .collect()
}

/// Whether a stage due after `position` can still run in sequence: it
/// cannot once any later position of the lifetime has finished (done or
/// failed), because what it would act on is no longer what the next task
/// started from. The driver records such a stage `skipped` instead.
pub fn out_of_sequence<'a>(position: u32, rows: impl IntoIterator<Item = &'a Trial>) -> bool {
    rows.into_iter().any(|t| {
        t.position.is_some_and(|p| p > position)
            && matches!(t.status, TrialStatus::Done | TrialStatus::Failed)
    })
}

/// D12's refusal, and `setup`'s rule for a workspace applied to the store:
/// a trial home that *is* the real home would learn into it, and one that
/// *contains* it would jail a run over the owner's tokens and transcripts.
pub fn refuse_unsafe_home(home: &Path, real: &Path) -> Result<()> {
    let norm = |p: &Path| -> PathBuf {
        // Lexical, not canonical: the trial home may not exist yet, and a
        // symlink under the real home is still under the real home.
        let mut out = PathBuf::new();
        for c in p.components() {
            match c {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    };
    let (home, real) = (norm(home), norm(real));
    anyhow::ensure!(
        home != real,
        "the trial home {} is the real mecha home; an experiment never runs in it",
        home.display()
    );
    anyhow::ensure!(
        !real.starts_with(&home),
        "the trial home {} contains the real mecha home {}; a run jailed there would hold the owner's tokens and transcripts",
        home.display(),
        real.display()
    );
    Ok(())
}

/// Is this home an experiment home — does it carry the marker?
pub fn is_experiment_home(home: &Path) -> bool {
    home.join(HOME_MARKER).is_file()
}

/// [`is_experiment_home`] for the home this process runs in. `false` when
/// the home cannot be resolved: unknown is never admitted.
pub fn in_experiment_home() -> bool {
    crate::work::mecha_home()
        .map(|h| is_experiment_home(&h))
        .unwrap_or(false)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("installing {}", path.display()))?;
    Ok(())
}

// ─── The child invocation ───────────────────────────────────────────────────

/// Which trial a run is one actor of — carried on `RunConfig::experiment`
/// (§4), so a session can say which trial it belonged to without a scan of
/// the experiment store. Set by the runner on the child's environment as
/// [`EXPERIMENT_REF_ENV`]; nothing else sets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentRef {
    pub exp_id: String,
    pub trial_id: String,
    pub arm: String,
    /// The actor's producer name — for `single`, the trial id.
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub task: String,
    pub repetition: u32,
    pub condition_hash: String,
}

pub const EXPERIMENT_REF_ENV: &str = "MECHA_EXPERIMENT_REF";

impl ExperimentRef {
    /// The reference this process was started under, if any. Read once per
    /// `RunConfig::of`; a malformed value is a warning and `None`, never a
    /// guess at a trial.
    pub fn from_env() -> Option<ExperimentRef> {
        let raw = std::env::var(EXPERIMENT_REF_ENV).ok()?;
        if raw.is_empty() {
            return None;
        }
        match serde_json::from_str(&raw) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!("{EXPERIMENT_REF_ENV} ignored: {e}");
                None
            }
        }
    }
}

/// How an arm reaches a child `mecha run`: the trial home's `config.toml`
/// carries the operator's whole posture with the arm's switches applied,
/// and the levers that are CLI-only become `--no-*` flags. Pure — the
/// runner writes the file and spawns the process.
///
/// **The machine's posture travels, the arm varies the closed set.** The
/// child config starts from the operator's config, not from defaults: the
/// sandbox and security sections, the approval `[[rule]]`s and `[approval]`
/// (the first cut dropped them, so every trial ran with the operator's
/// `forbid` list gone *and* `--yes` — the silently-degrading-guard shape,
/// and the exact opposite of what refusing the `approval_rules` lever
/// promises; found on review), the `[mcp]`, `[[hook]]` and `[outbox]`
/// sections a lever left on needs something to be on *of*, search
/// backends, subagent profiles. Every provider's inline `api_key` is
/// scrubbed — the variable `api_key_env` names passes through the child's
/// environment (`passthrough`), and a secret does not belong in a store a
/// trial's artifacts get exported from. The trial's seed pins the default
/// provider.
#[derive(Debug, Clone)]
pub struct ChildInvocation {
    pub config: crate::config::Config,
    pub flags: Vec<String>,
    /// Environment variables the child needs beyond the base set: every
    /// provider's `api_key_env`.
    pub passthrough: Vec<String>,
}

pub fn child_invocation(
    real: &crate::config::Config,
    arm: &Arm,
    seed: Option<u64>,
) -> Result<ChildInvocation> {
    let levers_off = arm.resolve_levers()?;
    let mut config = real.clone();
    // The arm's provider, or the operator's default: the model axis.
    if let Some(p) = &arm.provider {
        config.default_provider = p.clone();
    }
    anyhow::ensure!(
        config.providers.contains_key(&config.default_provider),
        "arm names provider `{}` but the operator's config has no [providers.{}]",
        config.default_provider,
        config.default_provider
    );
    let mut passthrough = Vec::new();
    let default = config.default_provider.clone();
    for (name, provider) in config.providers.iter_mut() {
        provider.api_key = None;
        if let Some(env) = &provider.api_key_env {
            passthrough.push(env.clone());
        }
        if name == &default {
            if seed.is_some() {
                provider.seed = seed;
            }
            if let Some(m) = &arm.model {
                provider.model = Some(m.clone());
            }
        }
    }
    // The other inline-secret surface: a search backend's key travels by
    // its variable, never by value (found on review).
    for backend in config.search.iter_mut() {
        backend.api_key = None;
        if let Some(env) = &backend.api_key_env {
            passthrough.push(env.clone());
        }
    }
    // An MCP server's `env` is where a token is written down, and the
    // server cannot start without it — so it rides into the trial home only
    // when the arm keeps MCP on; an arm with the lever off starts no server
    // and carries none of it. When it does ride, it is the same secret in
    // the same user's home one directory over, and `export` never reads
    // the config file.
    if levers_off.contains(&Lever::Mcp) {
        for server in config.mcp.iter_mut() {
            server.env.clear();
        }
    }
    // Every store path is the home's (D12). An operator's `[outbox] dir`
    // rode in verbatim, so a trial's drafts would have staged into the
    // real outbox and the principal read a store the verbs it emitted did
    // not write (found on review); the same for the skills and messages
    // directories. Cleared, each store resolves under `MECHA_HOME`.
    config.outbox.dir = None;
    config.skills.dir = None;
    config.messages.dir = None;
    let mut flags = Vec::new();
    for lever in levers_off {
        match lever {
            Lever::StepEscalation => config.agent.step_escalation = false,
            Lever::Boredom => config.agent.boredom = false,
            Lever::CompactValidate => config.agent.compact_validate = false,
            Lever::PredictiveCompaction => config.agent.predictive_compaction = false,
            Lever::CarriedState => config.agent.carried_state = false,
            Lever::Messages => {
                config.messages.enabled = false;
                flags.push("--no-messages".into());
            }
            Lever::Mcp => flags.push("--no-mcp".into()),
            Lever::LearnedRules => flags.push("--no-learned-rules".into()),
            Lever::Hooks => flags.push("--no-hooks".into()),
            Lever::Outbox => flags.push("--no-outbox".into()),
            Lever::Fallback => flags.push("--no-fallback".into()),
            Lever::Skills => flags.push("--no-skills".into()),
            Lever::Charter => flags.push("--no-charter".into()),
            Lever::CompactTool => flags.push("--no-compact-tool".into()),
            Lever::ApprovalRules => unreachable!("refused at load"),
        }
    }
    for spec in &arm.overrides {
        let change = crate::harness::parse_change(spec)?;
        change.apply_to_agent(&mut config.agent)?;
    }
    // The one stage lever that is a config switch rather than a verb: it
    // rides in the trial home's config, where `harness ruminate` reads it.
    if arm.resolve_stages()?.contains(&StageLever::SensorsInBrief) {
        config.agent.sensors_in_brief = false;
    }
    Ok(ChildInvocation {
        config,
        flags,
        passthrough,
    })
}

/// A trial home's own accepted harness overrides, relative to the home.
pub const HOME_OVERRIDES: &str = "learning/harness/overrides.toml";

/// The principal's scripted refusals for the task about to run, in the
/// trial home; the run child reads it through `tool::DENIALS_FILE_ENV`.
pub fn denials_file(home: &Path) -> PathBuf {
    home.join("principal").join("denials.toml")
}

/// The keys the home's own loop has moved (`HOME_OVERRIDES`), for a caller
/// that must treat them as the arm's pins without rendering a config —
/// the driver's act children carry a case's ceilings under the same rule
/// the run child does.
pub fn home_moved_keys(home: &Path) -> Result<Vec<String>> {
    let path = home.join(HOME_OVERRIDES);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let root = path.parent().expect("under the home");
    Ok(crate::harness::HarnessStore::open(root)?
        .overrides()?
        .into_iter()
        .map(|o| o.key)
        .collect())
}

/// Write the refusals the principal scripted, or remove the file when it
/// scripted none — a stale file would refuse the next task on the last
/// one's word.
pub fn write_denials(home: &Path, rules: &[crate::tool::DenialRule]) -> Result<()> {
    let path = denials_file(home);
    if rules.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().expect("under the home"))?;
    let file = crate::tool::DenialsFile {
        rules: rules.to_vec(),
    };
    write_atomic(&path, toml::to_string_pretty(&file)?.as_bytes())
}

/// Fold the home's *own* accepted overrides — what its stages accepted,
/// never the machine's, which `seed_home` drops — into the config the
/// next task runs under. A `lifetime`'s driver only: a single runs no
/// stage and has nothing to fold. The child's loader applies `overrides.toml` beneath every
/// file layer, and the rendered `config.toml` names every `[agent]` key,
/// so without this a change `harness ruminate` accepted inside the home
/// never reached a task — the one stage with an effect today measured as
/// nothing, with the ledger saying it ran (found on review). The arm's own
/// `overrides` are re-applied last: they are the design, and a lifetime
/// whose loop could move the treatment key would drift off its hash.
pub fn fold_home_overrides(
    config: &mut crate::config::Config,
    home: &Path,
    arm: &Arm,
) -> Result<Vec<String>> {
    // Strict, unlike the child's own loader: there a knob that fails to
    // apply must not stop every run from starting, but here the fold *is*
    // what makes a stage measurable, and a torn file applying nothing
    // would read as "rumination had no effect" with the ledger saying it
    // ran (found on review). The task's row carries the error instead.
    // Returns the keys the home's file moved: a key the loop moved is the
    // treatment for the next task exactly as an arm's pin is, and the
    // driver must not let a case's own ceiling flag override it — a flag
    // beats the rendered config, and 37 of the shipped cases carry one
    // (found on review).
    let mut moved = Vec::new();
    let path = home.join(HOME_OVERRIDES);
    if path.exists() {
        let root = path.parent().expect("under the home");
        let accepted = crate::harness::HarnessStore::open(root)?
            .overrides()
            .context("the trial home's accepted overrides could not be read, and the next task would have run without what a stage accepted")?;
        for ov in accepted {
            crate::harness::parse_change(&format!("{}={}", ov.key, ov.value))
                .with_context(|| format!("accepted override `{}` in {}", ov.key, path.display()))?
                .apply_to_agent(&mut config.agent)?;
            moved.push(ov.key);
        }
    }
    for spec in &arm.overrides {
        crate::harness::parse_change(spec)?.apply_to_agent(&mut config.agent)?;
    }
    Ok(moved)
}

/// The stores a lever left *on* reads: the learning store (rules and
/// reflections), the skills directory, the charter. A fresh trial home has
/// none, so `full` would have meant "the machine's `[agent]` switches and
/// nothing else" (found on review). Seeded once, when the arm's home is
/// first created, from the real home — a snapshot, never written back, so
/// `full` means the harness as this machine had it when the arm started
/// and a trial's `learn` lands in the copy.
pub const SEEDED: [&str; 3] = ["learning", "skills", "charter.toml"];

pub fn seed_home(real: &Path, home: &Path) -> Result<()> {
    for name in SEEDED {
        let from = real.join(name);
        let to = home.join(name);
        if !from.exists() || to.exists() {
            continue;
        }
        copy_tree(&from, &to)
            .with_context(|| format!("seeding {} into {}", name, home.display()))?;
    }
    // The machine's own accepted overrides ride in `learning/`, and they
    // are already in the rendered config at their real precedence —
    // beneath the operator's file, where the loader puts them. A copy in
    // the home would be folded *above* that file by `fold_home_overrides`,
    // inverting the precedence for every arm under an unchanged hash
    // (found on review). Dropped, so the home's file holds only what the
    // home's own stages accept.
    let seeded = home.join(HOME_OVERRIDES);
    if seeded.exists() {
        std::fs::remove_file(&seeded)
            .with_context(|| format!("dropping the seeded {}", seeded.display()))?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;
    }
    Ok(())
}

// ─── The judge ──────────────────────────────────────────────────────────────

/// One treatment arm's verdict against the control.
#[derive(Debug, Clone, Serialize)]
pub struct ArmJudgement {
    pub arm: String,
    pub metric: ExpMetric,
    pub pairs: usize,
    pub selection: usize,
    pub holdout: usize,
    pub judgement: Judgement,
    /// The treatment arm's and the control's stage runs, over every
    /// lifetime of each. Zero everywhere on a `single`. A broken stage on
    /// either side holds the verdict at *propose*: the ledger is the
    /// evidence that the treatment occurred, and a stage that failed or
    /// was interrupted is a treatment not known to have run as designed —
    /// the rule "a trial that cannot answer the metric drops its pair
    /// rather than counting as zero", one level up (found on review).
    pub stages: StageHealth,
    pub control_stages: StageHealth,
    /// Ledger lines on the store that did not parse. A torn line has no
    /// arm, so it holds every verdict: a line that could not be read is
    /// not evidence the stage ran as designed.
    pub unreadable_stage_lines: usize,
}

/// A finished control trial and the treatment trial on the same episode.
struct TrialPair<'a> {
    episode: String,
    control: &'a Trial,
    treatment: &'a Trial,
    control_cost: f64,
    treatment_cost: f64,
}

/// Pair every treatment arm's finished trials with the control's by
/// (task, seed, repetition), draw the holdout uniformly with the manifest's
/// split seed (`judge_drawn`'s rule — the pool here is uniform by
/// construction, but the draw is seeded so the same manifest judges the
/// same way twice), and judge each arm through the gate. A trial that
/// cannot answer the metric drops its pair; an arm with no pairs is
/// reported with none rather than omitted.
pub fn judge(
    manifest: &Manifest,
    trials: &[Trial],
    stages: &[StageRun],
    unreadable_stage_lines: usize,
) -> Vec<ArmJudgement> {
    let health_of = |arm: &str| {
        let own: Vec<StageRun> = stages.iter().filter(|r| r.arm == arm).cloned().collect();
        stage_health(&own)
    };
    // A measurement has no control and nothing to judge: every arm is read
    // for what it measured, never paired.
    let Some(control_name) = &manifest.control else {
        return Vec::new();
    };
    let control: BTreeMap<String, &Trial> = trials
        .iter()
        .filter(|t| &t.arm == control_name && t.status == TrialStatus::Done)
        .map(|t| (episode_key(t), t))
        .collect();
    let mut out = Vec::new();
    for (name, arm) in &manifest.arms {
        if name == control_name {
            continue;
        }
        let Some(prediction) = &arm.prediction else {
            continue;
        };
        let metric = prediction.metric;
        let mut pairs: Vec<TrialPair<'_>> = trials
            .iter()
            .filter(|t| &t.arm == name && t.status == TrialStatus::Done)
            .filter_map(|t| {
                let c = control.get(&episode_key(t))?;
                Some(TrialPair {
                    episode: episode_key(t),
                    control: c,
                    treatment: t,
                    control_cost: metric.cost(c)?,
                    treatment_cost: metric.cost(t)?,
                })
            })
            .collect();
        pairs.sort_by(|a, b| a.episode.cmp(&b.episode));
        let n = pairs.len();
        let holdout_n = if n == 0 {
            0
        } else {
            ((n as u64).div_ceil(manifest.holdout_in) as usize)
                .max(crate::candidate::MIN_HOLDOUT_PAIRS)
                .min(n)
        };
        let drawn: Vec<usize> =
            crate::sample::take_uniform((0..n).collect(), manifest.split_seed, holdout_n);
        let (mut selection, mut holdout) = (Vec::new(), Vec::new());
        for (i, p) in pairs.iter().enumerate() {
            if drawn.contains(&i) {
                holdout.push(p);
            } else {
                selection.push(p);
            }
        }
        let judgement = crate::candidate::judge_slices(
            ChangeClass::Config,
            &selection,
            &holdout,
            |p| (p.episode.as_str(), p.control_cost, p.treatment_cost),
            |p| {
                (
                    p.control
                        .stats
                        .as_ref()
                        .map_or(0, |s| u64::from(s.tool_calls)),
                    p.treatment
                        .stats
                        .as_ref()
                        .map_or(0, |s| u64::from(s.tool_calls)),
                )
            },
        );
        let stages = health_of(name);
        let control_stages = health_of(control_name);
        let mut judgement = judgement;
        let broken = stages.broken() + control_stages.broken() + unreadable_stage_lines;
        // Both directions: a treatment not known to have occurred can no
        // more be refuted than confirmed, and a confident reject over two
        // arms that both failed to ruminate would read as evidence against
        // it (found on review).
        if broken > 0
            && matches!(
                judgement.disposition,
                crate::candidate::Disposition::Accept | crate::candidate::Disposition::Reject(_)
            )
        {
            judgement.disposition = crate::candidate::Disposition::Propose(format!(
                "{broken} stage line(s) failed, interrupted, unreadable, or in a status this build cannot read across this arm's and the control's lifetimes; the treatment is not known to have run as designed"
            ));
        }
        out.push(ArmJudgement {
            arm: name.clone(),
            metric,
            pairs: n,
            selection: selection.len(),
            holdout: holdout.len(),
            judgement,
            stages,
            control_stages,
            unreadable_stage_lines,
        });
    }
    out
}

fn episode_key(t: &Trial) -> String {
    format!(
        "{}|{}|{}",
        t.task,
        t.seed.map(|s| s.to_string()).unwrap_or_default(),
        t.repetition
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
name = "levers"
control = "full"
split_seed = 7
seeds = [1, 2]
repetitions = 1

[tasks]
cases = "eval/cases.jsonl"
fixture = "eval/workspace"

[arms.full]
preset = "full"

[arms.bare]
preset = "bare"
[arms.bare.prediction]
metric = "failure"
rationale = "everything off should fail more"

[arms.quiet]
levers_off = ["boredom", "compact_validate"]
overrides = ["max_turns=20"]
[arms.quiet.prediction]
metric = "turns"
rationale = "no notice, fewer turns"
"#;

    #[test]
    fn a_manifest_loads_and_enumerates_its_trials_in_a_stable_order() {
        let m = Manifest::parse(MANIFEST).unwrap();
        assert_eq!(m.control.as_deref(), Some("full"));
        assert_eq!(m.arms["bare"].resolve_levers().unwrap(), Lever::bare(&[]));
        assert_eq!(
            m.arms["quiet"].resolve_levers().unwrap(),
            vec![Lever::Boredom, Lever::CompactValidate],
            "in ALL's order, whatever the manifest's"
        );
        let tasks = vec!["b".to_string(), "a".to_string()];
        let trials = m.trials(&tasks, "local", "m");
        // 3 arms × 2 tasks × 2 seeds × 1 rep
        assert_eq!(trials.len(), 12);
        assert_eq!(
            trials[0].id, "bare__b__s1__r1",
            "arms sorted, tasks as given"
        );
        assert!(trials.iter().all(|t| t.status == TrialStatus::Pending));
        // Same arm, same seed: same hash across tasks; a different seed or
        // arm: a different hash.
        let h = |arm: &str, seed: u64, task: &str| {
            trials
                .iter()
                .find(|t| t.arm == arm && t.seed == Some(seed) && t.task == task)
                .unwrap()
                .condition_hash
                .clone()
        };
        assert_eq!(h("bare", 1, "a"), h("bare", 1, "b"));
        assert_ne!(h("bare", 1, "a"), h("bare", 2, "a"));
        assert_ne!(h("bare", 1, "a"), h("quiet", 1, "a"));
        assert_eq!(m.trials(&tasks, "local", "m")[3].id, trials[3].id, "stable");
    }

    /// The load-time rules, each of which is a design defect the runner
    /// must never get as far as running: no control, a control that
    /// predicts, a treatment that does not, an unknown lever, the rules
    /// lifted, a bad override.
    #[test]
    fn a_manifest_that_breaks_a_rule_does_not_load() {
        let bad = |edit: &dyn Fn(String) -> String, needle: &str| {
            let text = edit(MANIFEST.to_string());
            // `{:#}`: the rule's own words sit under an arm-naming context.
            let err = format!("{:#}", Manifest::parse(&text).unwrap_err());
            assert!(err.contains(needle), "{needle}: {err}");
        };
        bad(
            &|t| t.replace("control = \"full\"", "control = \"nope\""),
            "names no arm",
        );
        bad(
            &|t| {
                t.replace(
                    "[arms.full]\npreset = \"full\"",
                    "[arms.full]\npreset = \"full\"\n[arms.full.prediction]\nmetric = \"turns\"\nrationale = \"x\"",
                )
            },
            "must not carry a prediction",
        );
        bad(
            &|t| {
                t.replace("[arms.bare.prediction]\nmetric = \"failure\"\nrationale = \"everything off should fail more\"", "")
            },
            "carries no prediction",
        );
        bad(
            &|t| t.replace("\"boredom\"", "\"appraiser\""),
            "not a lever",
        );
        bad(
            &|t| t.replace("\"boredom\"", "\"approval_rules\""),
            "operator's standing word",
        );
        bad(&|t| t.replace("max_turns=20", "max_turns=0"), "at least 1");
        bad(
            &|t| t.replace("name = \"levers\"", "name = \"a/b\""),
            "name",
        );
    }

    #[test]
    fn a_child_invocation_carries_the_machines_posture_and_the_arm_and_no_secret() {
        let m = Manifest::parse(MANIFEST).unwrap();
        let mut real = crate::config::Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        real.providers.insert(
            "local".into(),
            crate::config::ProviderConfig {
                kind: "local".into(),
                model: Some("m".into()),
                api_key: Some("secret".into()),
                api_key_env: Some("LOCAL_KEY".into()),
                ..Default::default()
            },
        );
        real.rules.push(crate::policy::RuleConfig {
            tool: "shell".into(),
            decision: crate::policy::RuleDecision::Forbid,
            ..Default::default()
        });
        let quiet = child_invocation(&real, &m.arms["quiet"], Some(3)).unwrap();
        assert!(!quiet.config.agent.boredom);
        assert!(!quiet.config.agent.compact_validate);
        assert_eq!(quiet.config.agent.max_turns, 20, "the override landed");
        assert!(quiet.flags.is_empty(), "both levers are config switches");
        let p = &quiet.config.providers["local"];
        assert_eq!(p.seed, Some(3), "the trial's seed pins the provider");
        assert_eq!(p.api_key, None, "no secret in the store");
        assert!(
            quiet.passthrough.iter().any(|v| v == "LOCAL_KEY"),
            "the key's variable travels: {:?}",
            quiet.passthrough
        );
        assert_eq!(
            quiet.config.rules.len(),
            1,
            "the operator's forbid travels — refusing the lever must mean something"
        );

        // The other secret surfaces: a search key travels by its variable;
        // an MCP server's env rides only when the arm keeps MCP on.
        real.search.push(crate::config::SearchBackendConfig {
            kind: "exa".into(),
            api_key: Some("s3".into()),
            api_key_env: Some("EXA_KEY".into()),
            ..Default::default()
        });
        real.mcp.push(crate::config::McpServerConfig {
            name: "graph".into(),
            env: [("TOKEN".to_string(), "t".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        });
        let quiet = child_invocation(&real, &m.arms["quiet"], None).unwrap();
        assert_eq!(quiet.config.search[0].api_key, None);
        assert!(quiet.passthrough.iter().any(|v| v == "EXA_KEY"));
        assert_eq!(
            quiet.config.mcp[0].env.len(),
            1,
            "MCP on: the server needs its env"
        );

        let bare = child_invocation(&real, &m.arms["bare"], None).unwrap();
        assert!(
            bare.config.mcp[0].env.is_empty(),
            "MCP off: no server starts, no token rides"
        );
        for flag in [
            "--no-mcp",
            "--no-learned-rules",
            "--no-hooks",
            "--no-outbox",
            "--no-fallback",
            "--no-messages",
            "--no-skills",
            "--no-charter",
            "--no-compact-tool",
        ] {
            assert!(
                bare.flags.iter().any(|f| f == flag),
                "{flag}: {:?}",
                bare.flags
            );
        }
        assert!(!bare.config.agent.predictive_compaction);
        assert!(!bare.config.agent.carried_state);
        assert!(!bare.config.messages.enabled);
        assert_eq!(
            bare.config.providers["local"].seed, None,
            "unseeded stays unseeded"
        );
        let text = toml::to_string(&bare.config).unwrap();
        let back: crate::config::Config = toml::from_str(&text).unwrap();
        assert_eq!(back.agent.max_turns, bare.config.agent.max_turns);
        assert_eq!(back.rules.len(), 1);
    }

    /// The stores a lever reads are seeded once from the real home, never
    /// written back: `full` means the harness as this machine has it.
    #[test]
    fn an_arm_home_is_seeded_from_the_real_stores_once() {
        let root = std::env::temp_dir().join(format!("mecha-exp-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let real = root.join("real");
        std::fs::create_dir_all(real.join("learning")).unwrap();
        std::fs::create_dir_all(real.join("skills").join("deploy")).unwrap();
        std::fs::write(real.join("learning").join("rules.jsonl"), b"{}\n").unwrap();
        std::fs::write(
            real.join("skills").join("deploy").join("SKILL.md"),
            b"# deploy\n",
        )
        .unwrap();
        std::fs::write(real.join("charter.toml"), b"[[line]]\n").unwrap();
        std::fs::write(real.join("config.toml"), b"default_provider = \"x\"\n").unwrap();
        let home = root.join("home");
        seed_home(&real, &home).unwrap();
        assert!(home.join("learning").join("rules.jsonl").is_file());
        assert!(home
            .join("skills")
            .join("deploy")
            .join("SKILL.md")
            .is_file());
        assert!(home.join("charter.toml").is_file());
        assert!(
            !home.join("config.toml").exists(),
            "the config is the arm's, not the machine's"
        );
        // Once: a later seed does not overwrite what the trial home has.
        std::fs::write(home.join("charter.toml"), b"[[line]]\n[[line]]\n").unwrap();
        seed_home(&real, &home).unwrap();
        assert_eq!(
            std::fs::read(home.join("charter.toml")).unwrap(),
            b"[[line]]\n[[line]]\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The other axis: an arm that names a provider or a model runs
    /// against it, its trials hash to it, and an arm naming a provider the
    /// operator has not configured is refused where the config is known.
    #[test]
    fn an_arm_can_name_its_model_and_the_hash_follows() {
        let text = MANIFEST.replace(
            "[arms.quiet]\nlevers_off",
            "[arms.quiet]\nprovider = \"small\"\nmodel = \"tiny\"\nlevers_off",
        );
        let m = Manifest::parse(&text).unwrap();
        let trials = m.trials(&["a".to_string()], "local", "m");
        let of = |arm: &str| {
            trials
                .iter()
                .find(|t| t.arm == arm && t.seed == Some(1))
                .unwrap()
                .condition_hash
                .clone()
        };
        assert_ne!(of("quiet"), of("full"));
        let mut real = crate::config::Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        real.providers.insert(
            "local".into(),
            crate::config::ProviderConfig {
                kind: "local".into(),
                model: Some("m".into()),
                ..Default::default()
            },
        );
        assert!(child_invocation(&real, &m.arms["quiet"], None)
            .unwrap_err()
            .to_string()
            .contains("no [providers.small]"));
        real.providers.insert(
            "small".into(),
            crate::config::ProviderConfig {
                kind: "local".into(),
                model: Some("big".into()),
                ..Default::default()
            },
        );
        let inv = child_invocation(&real, &m.arms["quiet"], Some(2)).unwrap();
        assert_eq!(inv.config.default_provider, "small");
        assert_eq!(inv.config.providers["small"].model.as_deref(), Some("tiny"));
        assert_eq!(inv.config.providers["small"].seed, Some(2));
        assert_eq!(
            inv.config.providers["local"].seed, None,
            "the seed pins the arm's provider only"
        );
    }

    /// A manifest with no control is a measurement: one arm or several, none
    /// predicting, nothing judged — what a scorecard is. A prediction with
    /// no control to measure it against is refused, and so is a comparison
    /// with one arm.
    #[test]
    fn a_manifest_without_a_control_is_a_measurement() {
        let tasks = Tasks {
            cases: "eval/cases.jsonl".into(),
            fixture: "eval/workspace".into(),
            ids: Vec::new(),
            tags: Vec::new(),
        };
        let bare = Arm {
            preset: Some(Preset::Bare),
            model: Some("m".into()),
            ..Arm::default()
        };
        let m = Manifest::one_arm("eval-x", "bare", bare.clone(), tasks.clone(), 1).unwrap();
        assert_eq!(m.control, None);
        assert!(judge(&m, &[], &[], 0).is_empty(), "nothing to judge");
        let text = toml::to_string_pretty(&m).unwrap();
        let back = Manifest::parse(&text).unwrap();
        assert_eq!(back.control, None);
        assert_eq!(back.arms["bare"].model.as_deref(), Some("m"));

        let predicted = Arm {
            prediction: Some(Prediction {
                metric: ExpMetric::Failure,
                rationale: "x".into(),
            }),
            ..bare.clone()
        };
        assert!(Manifest::one_arm("eval-x", "bare", predicted, tasks.clone(), 1).is_err());

        // Several models, no control: a bake-off, each arm read alone.
        let text = MANIFEST
            .replace("control = \"full\"\n", "")
            .replace("[arms.bare.prediction]\nmetric = \"failure\"\nrationale = \"everything off should fail more\"\n", "")
            .replace("[arms.quiet.prediction]\nmetric = \"turns\"\nrationale = \"no notice, fewer turns\"\n", "");
        let bake = Manifest::parse(&text).unwrap();
        assert_eq!(bake.control, None);
        assert_eq!(bake.arms.len(), 3);
        assert!(
            judge(&bake, &[], &[], 0).is_empty(),
            "a bake-off judges nothing either"
        );
        // And a comparison still needs the control and a treatment arm: one
        // arm *with* a control is the rule the refactor could have dropped.
        let one = "name = \"eval-one\"\ncontrol = \"bare\"\nsplit_seed = 1\n\
                   [arms.bare]\npreset = \"bare\"\n\
                   [tasks]\ncases = \"eval/cases.jsonl\"\nfixture = \"eval/workspace\"\n";
        let e = Manifest::parse(one).unwrap_err().to_string();
        assert!(e.contains("leave `control` unset for a measurement"), "{e}");
    }

    /// `levers_on` after the preset: the add-one-to-bare design, and how
    /// `--ab-rules` is spelled. The rules refusal reaches it too.
    #[test]
    fn levers_on_reopens_a_preset_and_the_rules_refusal_still_holds() {
        let arm = Arm {
            preset: Some(Preset::Bare),
            levers_on: vec!["learned_rules".into()],
            ..Arm::default()
        };
        let off = arm.resolve_levers().unwrap();
        assert!(!off.contains(&Lever::LearnedRules));
        assert!(!off.contains(&Lever::ApprovalRules), "never in a preset");
        assert_eq!(off.len(), Lever::ALL.len() - 2);
        let both = Arm {
            levers_off: vec!["boredom".into()],
            levers_on: vec!["boredom".into()],
            ..Arm::default()
        };
        assert!(
            both.resolve_levers().unwrap().is_empty(),
            "on wins over off"
        );
        let bad = Arm {
            levers_on: vec!["approval_rules".into()],
            ..Arm::default()
        };
        assert!(bad.resolve_levers().is_err());
    }

    /// A front-end's two-arm design is a manifest like any other: valid,
    /// stored, and its holdout draw fixed by the treatment's description so
    /// a rerun holds out the same tasks. A lever both arms share is on both
    /// arms' records and moves the hash.
    #[test]
    fn a_two_arm_design_is_a_manifest_with_a_stable_split() {
        let tasks = Tasks {
            cases: "eval/cases.jsonl".into(),
            fixture: "eval/workspace".into(),
            ids: Vec::new(),
            tags: Vec::new(),
        };
        let treatment = Arm {
            preset: Some(Preset::Bare),
            overrides: vec!["max_turns=40".into()],
            prediction: Some(Prediction {
                metric: ExpMetric::Failure,
                rationale: "--ab-config".into(),
            }),
            ..Arm::default()
        };
        let mk = |name: &str, t: Arm, shared: &[String]| {
            Manifest::two_arm(name, "treatment", t, tasks.clone(), 3, 1, shared, &[]).unwrap()
        };
        let a = mk("eval-ab", treatment.clone(), &[]);
        let b = mk("eval-ab-again", treatment.clone(), &[]);
        assert_eq!(a.control.as_deref(), Some("bare"));
        assert_eq!(
            a.split_seed, b.split_seed,
            "the same delta draws the same holdout"
        );
        let other = mk(
            "eval-ab",
            Arm {
                preset: Some(Preset::Bare),
                overrides: vec!["max_turns=50".into()],
                prediction: Some(Prediction {
                    metric: ExpMetric::Failure,
                    rationale: "x".into(),
                }),
                ..Arm::default()
            },
            &[],
        );
        assert_ne!(a.split_seed, other.split_seed);
        let with_mcp = mk("eval-ab-mcp", treatment, &["mcp".to_string()]);
        for arm in with_mcp.arms.values() {
            assert!(
                !arm.resolve_levers().unwrap().contains(&Lever::Mcp),
                "{arm:?}"
            );
        }
        let task = ["a".to_string()];
        assert_ne!(
            with_mcp.trials(&task, "p", "m")[0].condition_hash,
            a.trials(&task, "p", "m")[0].condition_hash,
            "the shared lever moves the hash"
        );
        let text = toml::to_string_pretty(&a).unwrap();
        let back = Manifest::parse(&text).unwrap();
        assert_eq!(back.arms.len(), 2);
        assert_eq!(back.arms["treatment"].overrides, vec!["max_turns=40"]);

        // The knobs both arms inherit are on both records; the treatment's
        // own value for a shared key wins, and its hash differs from the
        // control's by exactly that.
        let shared = ["max_turns=60".to_string(), "effort=high".to_string()];
        let m = Manifest::two_arm(
            "eval-ab-knobs",
            "treatment",
            Arm {
                preset: Some(Preset::Bare),
                overrides: vec!["max_turns=40".into()],
                prediction: Some(Prediction {
                    metric: ExpMetric::Failure,
                    rationale: "x".into(),
                }),
                ..Arm::default()
            },
            tasks.clone(),
            3,
            1,
            &[],
            &shared,
        )
        .unwrap();
        assert_eq!(m.arms["bare"].overrides, shared.to_vec());
        // The seed is the delta's alone — shared knobs and shared levers
        // leave it unchanged — and it fits a TOML integer, so the manifest
        // it is written into can be read back.
        // The same delta with nothing shared: `m` merged the shared knobs
        // into its treatment row, so the comparison arm is rebuilt from the
        // delta rather than read back off `m`.
        let plain = mk(
            "eval-ab-plain",
            Arm {
                preset: Some(Preset::Bare),
                overrides: vec!["max_turns=40".into()],
                prediction: Some(Prediction {
                    metric: ExpMetric::Failure,
                    rationale: "x".into(),
                }),
                ..Arm::default()
            },
            &[],
        );
        assert_eq!(
            m.split_seed, plain.split_seed,
            "shared knobs are not in the seed"
        );
        assert_eq!(
            with_mcp.split_seed, a.split_seed,
            "shared levers are not in the seed"
        );
        for delta in [
            "max_turns=1",
            "max_turns=2",
            "effort=low",
            "compact_at_tokens=9000",
        ] {
            let t = Arm {
                preset: Some(Preset::Bare),
                overrides: vec![delta.into()],
                prediction: Some(Prediction {
                    metric: ExpMetric::Failure,
                    rationale: "x".into(),
                }),
                ..Arm::default()
            };
            let d = mk("eval-ab-seed", t, &[]);
            assert!(d.split_seed <= i64::MAX as u64, "{delta}");
            let text = toml::to_string_pretty(&d).unwrap();
            assert_eq!(
                Manifest::parse(&text).unwrap().split_seed,
                d.split_seed,
                "{delta}"
            );
        }
        assert_eq!(
            m.arms["treatment"].overrides,
            vec!["effort=high".to_string(), "max_turns=40".to_string()],
            "the treatment's own max_turns replaces the shared one"
        );
    }

    #[test]
    fn a_home_that_is_or_contains_the_real_one_is_refused() {
        let real = Path::new("/home/x/.mecha");
        assert!(refuse_unsafe_home(Path::new("/home/x/.mecha"), real).is_err());
        assert!(refuse_unsafe_home(Path::new("/home/x"), real).is_err());
        assert!(
            refuse_unsafe_home(Path::new("/home/x/.mecha/experiments/e/../../"), real).is_err()
        );
        assert!(
            refuse_unsafe_home(Path::new("/home/x/.mecha/experiments/e/homes/a"), real).is_ok()
        );
        assert!(refuse_unsafe_home(Path::new("/tmp/e/homes/a"), real).is_ok());
    }

    fn done(arm: &str, task: &str, seed: u64, passed: bool, turns: u32) -> Trial {
        Trial {
            id: trial_id(arm, task, Some(seed), 1),
            arm: arm.into(),
            task: task.into(),
            seed: Some(seed),
            repetition: 1,
            condition_hash: "h".into(),
            status: TrialStatus::Done,
            session_id: None,
            started_at: None,
            finished_at: None,
            error: None,
            passed: Some(passed),
            checks: Vec::new(),
            stats: Some(RunStats {
                turns,
                tool_calls: 3,
                ..RunStats::default()
            }),
            position: None,
            lifetime: None,
        }
    }

    /// A lifetime plans one home per arm × seed × repetition and walks the
    /// sequence in order inside it: rows carry their position and home,
    /// share the arm's hash, and the stage levers off move the hash while
    /// an arm with every stage on hashes as its single-trial twin.
    #[test]
    fn a_lifetime_plans_one_home_per_arm_seed_and_repetition_in_sequence_order() {
        let text = r#"
name = "life"
kind = "lifetime"
control = "full"
split_seed = 3
seeds = [1, 2]
repetitions = 2
[schedule]
reflect = 1
learn = 2
validate = 2
ruminate = 0
[tasks]
cases = "eval/cases.jsonl"
fixture = "eval/workspace"
ids = ["b", "a", "c"]
[arms.full]
preset = "full"
[arms.deaf]
stages_off = ["ruminate", "sensors_in_brief"]
[arms.deaf.prediction]
metric = "failure"
rationale = "no rumination should fail more over the sequence"
"#;
        let m = Manifest::parse(text).unwrap();
        assert_eq!(m.kind, TrialKind::Lifetime);
        let ids: Vec<String> = ["b", "a", "c"].iter().map(|s| s.to_string()).collect();
        let rows = m.trials(&ids, "p", "m");
        assert_eq!(rows.len(), 2 * 2 * 2 * 3);
        let deaf: Vec<&Trial> = rows.iter().filter(|t| t.arm == "deaf").collect();
        assert_eq!(
            deaf.iter()
                .take(3)
                .map(|t| (t.position, t.task.as_str()))
                .collect::<Vec<_>>(),
            vec![(Some(1), "b"), (Some(2), "a"), (Some(3), "c")]
        );
        assert_eq!(deaf[0].lifetime.as_deref(), Some("deaf__s1__r1"));
        assert_eq!(deaf[3].lifetime.as_deref(), Some("deaf__s1__r2"));
        assert_eq!(deaf[6].lifetime.as_deref(), Some("deaf__s2__r1"));
        assert!(deaf[..3]
            .iter()
            .all(|t| t.condition_hash == deaf[0].condition_hash));
        // The stage levers are on the hash, in the closed set's order.
        let full = rows
            .iter()
            .find(|t| t.arm == "full" && t.seed == Some(1))
            .unwrap();
        assert_ne!(full.condition_hash, deaf[0].condition_hash);
        assert_eq!(
            m.arms["deaf"].resolve_stages().unwrap(),
            vec![StageLever::Ruminate, StageLever::SensorsInBrief]
        );
        assert_eq!(
            condition_hash(&[], &[], "p", "m", Some(1)),
            condition_hash_with_stages(&[], &[], "p", "m", Some(1), &[]),
            "every stage on hashes as the single-trial twin"
        );
        // The schedule says what is due after a position, in run order.
        assert_eq!(m.schedule.due_after(1), vec![StageLever::Reflect]);
        assert_eq!(
            m.schedule.due_after(2),
            vec![StageLever::Reflect, StageLever::Validate, StageLever::Learn],
            "validate measures before learn consumes — the nightly's order"
        );
        assert!(
            !m.schedule.due_after(4).contains(&StageLever::Ruminate),
            "0 is never"
        );
        assert_eq!(
            Schedule::default().due_after(10),
            vec![
                StageLever::Reflect,
                StageLever::Validate,
                StageLever::Learn,
                StageLever::Retire,
                StageLever::Ruminate
            ],
            "the design's default: every, fifth, fifth, fifth, tenth — the nightly's order"
        );
    }

    /// The lifetime-only rules refuse at load: a stage lever or a schedule
    /// on a single, an unknown stage name, a task twice in the sequence.
    #[test]
    fn the_lifetime_rules_are_enforced_at_load() {
        let single = MANIFEST.replace("[arms.full]", "[arms.full]\nstages_off = [\"learn\"]");
        let e = Manifest::parse(&single).unwrap_err().to_string();
        assert!(e.contains("`single` experiment"), "{e}");
        let scheduled = MANIFEST.replace("[tasks]", "[schedule]\nreflect = 0\n[tasks]");
        let e = Manifest::parse(&scheduled).unwrap_err().to_string();
        assert!(e.contains("is a lifetime's"), "{e}");
        let unknown = MANIFEST
            .replace(
                "control = \"full\"",
                "kind = \"lifetime\"\ncontrol = \"full\"",
            )
            .replace(
                "fixture = \"eval/workspace\"",
                "fixture = \"eval/workspace\"\nids = [\"a\"]",
            )
            .replace(
                "[arms.full]",
                "[arms.full]\nstages_off = [\"followup_staging\"]",
            );
        let unnamed = MANIFEST.replace(
            "control = \"full\"",
            "kind = \"lifetime\"\ncontrol = \"full\"",
        );
        let e = Manifest::parse(&unnamed).unwrap_err().to_string();
        assert!(e.contains("names its sequence"), "{e}");
        let sixth: StageLever = serde_json::from_str("\"followup_staging\"").unwrap();
        assert_eq!(
            sixth,
            StageLever::Unknown,
            "a ledger's stage this build cannot name"
        );
        assert_eq!(sixth.argv(), None);
        assert!(!StageLever::ALL.contains(&StageLever::Unknown));
        let e = format!("{:#}", Manifest::parse(&unknown).unwrap_err());
        assert!(
            e.contains("not a stage lever") && e.contains("sensors_in_brief"),
            "{e}"
        );
        let seeds = MANIFEST.replace("seeds = [1, 2]", "seeds = [1, 1]");
        let e = Manifest::parse(&seeds).unwrap_err().to_string();
        assert!(e.contains("seed `1` appears twice"), "{e}");
        for kind in ["single", "lifetime"] {
            let twice = MANIFEST
                .replace(
                    "control = \"full\"",
                    &format!("kind = \"{kind}\"\ncontrol = \"full\""),
                )
                .replace(
                    "fixture = \"eval/workspace\"",
                    "fixture = \"eval/workspace\"\nids = [\"a\", \"a\"]",
                );
            let e = Manifest::parse(&twice).unwrap_err().to_string();
            assert!(e.contains("appears twice"), "{kind}: {e}");
        }
        for name in StageLever::ALL {
            assert_eq!(StageLever::parse(name.as_str()), Some(name));
            let wire: StageLever = serde_json::from_str(&format!("\"{}\"", name.as_str())).unwrap();
            assert_eq!(wire, name, "the wire name is the lever's name");
        }
        assert_eq!(StageLever::SensorsInBrief.argv(), None);
        assert_eq!(
            StageLever::Learn.argv(),
            Some(&["learn", "--holdout", "0.25", "--auto"][..])
        );
        assert_eq!(
            StageLever::Validate.argv(),
            Some(&["validate", "--unprocessed-only"][..]),
            "the held-out measurement, as the nightly runs it"
        );
        assert_eq!(
            StageLever::Retire.argv(),
            Some(&["rules", "propose-retirements", "--apply"][..]),
            "the brake, as the nightly runs it"
        );
    }

    /// The ledger says what ran: due stages are the schedule's minus the
    /// arm's levers minus the lines already done after that position, a
    /// failed stage is due again, and a torn line is a finding.
    #[test]
    fn the_stage_ledger_decides_what_is_still_due() {
        let dir = std::env::temp_dir().join(format!("mecha-exp-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = ExperimentStore::open(&dir, "life").unwrap();
        let run = |stage: StageLever, after: u32, status: StageStatus| StageRun {
            lifetime: "full__r1".into(),
            arm: "full".into(),
            stage,
            after_position: after,
            attempt: 1,
            started_at: "t0".into(),
            finished_at: "t1".into(),
            status,
            exit_code: Some(0),
            error: None,
            point: None,
            acts: Vec::new(),
            refusals: Vec::new(),
        };
        store
            .record_stage(&run(StageLever::Reflect, 2, StageStatus::Done))
            .unwrap();
        store
            .record_stage(&run(StageLever::Learn, 2, StageStatus::Failed))
            .unwrap();
        let (ledger, torn) = store.stage_runs("full__r1").unwrap();
        assert_eq!((ledger.len(), torn), (2, 0));
        let schedule = Schedule {
            reflect: 1,
            learn: 2,
            validate: 2,
            retire: 0,
            ruminate: 0,
        };
        assert_eq!(
            stages_due(&schedule, 2, &[StageLever::Validate], &ledger),
            vec![StageLever::Learn],
            "reflect done, learn failed so due again, validate off, ruminate never"
        );
        assert_eq!(
            stages_due(&schedule, 1, &[], &ledger),
            vec![StageLever::Reflect]
        );
        assert!(stages_due(&schedule, 2, &StageLever::ALL, &ledger).is_empty());
        let (_, none) = store.stage_runs("nobody").unwrap();
        assert_eq!(none, 0);
        // A torn line and an unknown status are findings, never stages done.
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(store.stage_ledger("full__r1"))
            .unwrap();
        writeln!(f, "{{not json").unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&run(StageLever::Reflect, 3, StageStatus::Done))
                .unwrap()
                .replace("\"done\"", "\"paused\"")
        )
        .unwrap();
        let (ledger, torn) = store.stage_runs("full__r1").unwrap();
        assert_eq!((ledger.len(), torn), (3, 1));
        assert_eq!(ledger[2].status, StageStatus::Unknown);
        assert_eq!(
            stages_due(&schedule, 3, &[], &ledger),
            vec![StageLever::Reflect]
        );
        assert!(store
            .stage_log("full__r1", 3, StageLever::Reflect, 2)
            .ends_with("full__r1/003-reflect-a2.log"));
        assert_eq!(
            ExperimentStore::next_attempt(&ledger, 0, 2, StageLever::Learn),
            2,
            "one failed learn on the ledger: the rerun is attempt 2"
        );
        assert_eq!(
            ExperimentStore::next_attempt(&ledger, 0, 9, StageLever::Learn),
            1
        );
        assert_eq!(
            ExperimentStore::next_attempt(&ledger, 1, 9, StageLever::Learn),
            2,
            "a torn line may have been this pair's: never reuse its number"
        );
        // A running line no terminal line supersedes is an interrupted
        // stage; one a terminal line supersedes is that terminal line's.
        let mut running = run(StageLever::Ruminate, 4, StageStatus::Running);
        running.attempt = 3;
        store.record_stage(&running).unwrap();
        let (ledger, _) = store.stage_runs("full__r1").unwrap();
        let h = stage_health(&ledger);
        assert_eq!(
            (h.done, h.failed, h.interrupted, h.unknown),
            (1, 1, 1, 1),
            "reflect done, learn failed, the running ruminate interrupted, the paused line unknown: {h:?}"
        );
        assert_eq!(
            ExperimentStore::next_attempt(&ledger, 0, 4, StageLever::Ruminate),
            2,
            "the running line burns its attempt"
        );
        let mut settled = run(StageLever::Ruminate, 4, StageStatus::Done);
        settled.attempt = 3;
        store.record_stage(&settled).unwrap();
        let (ledger, _) = store.stage_runs("full__r1").unwrap();
        let h = stage_health(&ledger);
        assert_eq!((h.done, h.interrupted), (2, 0), "{h:?}");
        // The failed learn's rerun succeeds: the failure is superseded and
        // the stage is known to have run.
        assert_eq!(h.failed, 1);
        let mut learned = run(StageLever::Learn, 2, StageStatus::Done);
        learned.attempt = 2;
        store.record_stage(&learned).unwrap();
        let (ledger, _) = store.stage_runs("full__r1").unwrap();
        let h = stage_health(&ledger);
        assert_eq!((h.done, h.failed), (3, 0), "{h:?}");
        // An interrupted stage whose rerun succeeds on the next attempt is
        // that success, not an interruption forever.
        let mut cut = run(StageLever::Validate, 5, StageStatus::Running);
        cut.attempt = 1;
        store.record_stage(&cut).unwrap();
        let (ledger, _) = store.stage_runs("full__r1").unwrap();
        assert_eq!(stage_health(&ledger).interrupted, 1);
        let next = ExperimentStore::next_attempt(&ledger, 0, 5, StageLever::Validate);
        assert_eq!(next, 2, "the interrupted attempt is burnt");
        for status in [StageStatus::Running, StageStatus::Done] {
            let mut again = run(StageLever::Validate, 5, status);
            again.attempt = next;
            store.record_stage(&again).unwrap();
        }
        let (ledger, _) = store.stage_runs("full__r1").unwrap();
        let h = stage_health(&ledger);
        assert_eq!((h.interrupted, h.done), (0, 4), "{h:?}");
        // A stage the driver could no longer run in sequence: recorded
        // skipped, never due again, broken, and standing for its failure.
        store
            .record_stage(&run(StageLever::Reflect, 6, StageStatus::Failed))
            .unwrap();
        let mut skipped = run(StageLever::Reflect, 6, StageStatus::Skipped);
        skipped.attempt = 2;
        store.record_stage(&skipped).unwrap();
        let (ledger, _) = store.stage_runs("full__r1").unwrap();
        let h = stage_health(&ledger);
        assert_eq!(
            (h.skipped, h.failed, h.broken()),
            (1, 0, 2),
            "the skip stands for its failure; the paused line at 3 is the other broken one: {h:?}"
        );
        assert!(
            !stages_due(&schedule, 6, &[], &ledger).contains(&StageLever::Reflect),
            "a skipped stage is never due again"
        );
        let mut rows = vec![done("full", "a", 1, true, 3), done("full", "b", 1, true, 3)];
        rows[0].position = Some(6);
        rows[1].position = Some(7);
        assert!(
            out_of_sequence(6, &rows),
            "position 7 finished: a stage after 6 is out of sequence"
        );
        rows[1].status = TrialStatus::Pending;
        assert!(
            !out_of_sequence(6, &rows),
            "nothing later finished: still in sequence"
        );
        assert!(!out_of_sequence(7, &rows));
        assert_eq!(
            stages_due(&schedule, 2, &[], &ledger),
            vec![StageLever::Validate],
            "learn is done now; validate never ran after 2 and is still due"
        );
        let (all, torn) = store.all_stage_runs().unwrap();
        assert_eq!((all.len(), torn), (ledger.len(), 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A broken stage on either side of a comparison holds the verdict at
    /// propose: a treatment whose stages failed is not known to have run.
    #[test]
    fn a_broken_stage_holds_the_verdict_at_propose() {
        let m = Manifest::parse(MANIFEST).unwrap();
        let mut trials = Vec::new();
        for i in 0..12u64 {
            trials.push(done("full", &format!("t{i}"), 1, true, 10));
            trials.push(done("bare", &format!("t{i}"), 1, false, 10));
        }
        let clean = judge(&m, &trials, &[], 0);
        let bare = clean.iter().find(|v| v.arm == "bare").unwrap();
        assert_eq!(bare.stages, StageHealth::default());
        let failed = StageRun {
            lifetime: "bare__s1__r1".into(),
            arm: "bare".into(),
            stage: StageLever::Ruminate,
            after_position: 10,
            attempt: 1,
            started_at: "t0".into(),
            finished_at: "t1".into(),
            status: StageStatus::Failed,
            exit_code: Some(1),
            error: None,
            point: None,
            acts: Vec::new(),
            refusals: Vec::new(),
        };
        let held = judge(&m, &trials, std::slice::from_ref(&failed), 0);
        let bare_held = held.iter().find(|v| v.arm == "bare").unwrap();
        assert_eq!(bare_held.stages.failed, 1);
        assert!(
            matches!(
                bare.judgement.disposition,
                crate::candidate::Disposition::Reject(_)
            ),
            "the clean verdict is a reject (bare fails more): {:?}",
            bare.judgement.disposition
        );
        assert!(
            matches!(
                bare_held.judgement.disposition,
                crate::candidate::Disposition::Propose(_)
            ),
            "neither accepted nor rejected over a stage that did not run: {:?}",
            bare_held.judgement.disposition
        );
        // Unknown is never clean, and a torn line holds every verdict.
        let mut odd = failed.clone();
        odd.status = StageStatus::Unknown;
        let held = judge(&m, &trials, &[odd], 0);
        let v = held.iter().find(|v| v.arm == "bare").unwrap();
        assert_eq!((v.stages.unknown, v.stages.broken()), (1, 1));
        assert!(matches!(
            v.judgement.disposition,
            crate::candidate::Disposition::Propose(_)
        ));
        let held = judge(&m, &trials, &[], 1);
        let v = held.iter().find(|v| v.arm == "bare").unwrap();
        assert_eq!(v.unreadable_stage_lines, 1);
        assert!(matches!(
            v.judgement.disposition,
            crate::candidate::Disposition::Propose(_)
        ));
        assert_eq!(
            bare_held.judgement.selection, bare.judgement.selection,
            "the tallies are untouched"
        );
    }

    /// A trial home owns every store path: an operator's explicit outbox,
    /// skills or messages directory never rides into a child's config.
    #[test]
    fn a_child_home_owns_every_store_path() {
        let mut real = crate::config::Config::default();
        real.outbox.dir = Some("/home/me/.mecha/outbox".into());
        real.skills.dir = Some("/home/me/.mecha/skills".into());
        real.messages.dir = Some("/home/me/.mecha/messages".into());
        let child = child_invocation(&real, &Arm::default(), None)
            .unwrap()
            .config;
        assert_eq!(
            child.outbox.dir, None,
            "the real outbox would take a trial's drafts"
        );
        assert_eq!(child.skills.dir, None);
        assert_eq!(child.messages.dir, None);
    }

    /// The principal's contract: a `[principal]` is a lifetime's, its
    /// command is named, the closed verb set admits the owner's channels
    /// and nothing else, its answer parses strictly, and the refusals it
    /// scripts land in the home's denials file and clear on an empty list.
    #[test]
    fn the_principal_contract_is_closed_and_the_denials_file_follows_it() {
        let single = MANIFEST.replace("[tasks]", "[principal]\ncommand = [\"/bin/true\"]\n[tasks]");
        let e = Manifest::parse(&single).unwrap_err().to_string();
        assert!(e.contains("a `[principal]` is a lifetime's"), "{e}");
        let life = MANIFEST
            .replace(
                "control = \"full\"",
                "kind = \"lifetime\"\ncontrol = \"full\"",
            )
            .replace(
                "fixture = \"eval/workspace\"",
                "fixture = \"eval/workspace\"\nids = [\"a\"]",
            )
            .replace(
                "[tasks]",
                "[principal]\ncommand = [\"/usr/bin/env\", \"python3\", \"/x/gold.py\"]\n[tasks]",
            );
        let m = Manifest::parse(&life).unwrap();
        let p = m.principal.unwrap();
        assert_eq!(p.command[1], "python3");
        assert_eq!(p.timeout_secs, 600, "the default");
        let empty = life.replace(
            "command = [\"/usr/bin/env\", \"python3\", \"/x/gold.py\"]",
            "command = []",
        );
        assert!(Manifest::parse(&empty).is_err(), "no executable");
        // The verb set.
        let v = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert!(
            !allowed_verb(&v("outbox approve ob-1 --yes")),
            "a release executes the routed tool for real; it waits on fixture servers"
        );
        assert!(allowed_verb(&v("outbox reject ob-1 --reason no")));
        assert!(allowed_verb(&v("outbox edit ob-1 --body-file draft.txt")));
        assert!(allowed_verb(&v(
            "questions answer q-1 --unattended yes please"
        )));
        assert!(allowed_verb(&v("tasks set t-1 --status done")));
        assert!(
            !allowed_verb(&v("run do something")),
            "never a session of its own"
        );
        assert!(!allowed_verb(&v("learn --auto")), "never a rule");
        assert!(
            !allowed_verb(&v("outbox")),
            "a leading word alone is not a verb"
        );
        assert!(!allowed_verb(&[]));
        assert!(
            !allowed_verb(&v("outbox reject ob-1 --workspace /tmp")),
            "the workspace is the driver's"
        );
        assert!(!allowed_verb(&v(
            "tasks set t-1 --status done --provider other"
        )));
        assert!(!allowed_verb(&v("questions answer q-1 --model=x yes")));
        assert!(
            !allowed_verb(&v("tasks set t-1 --no-learned-rules")),
            "levers are the arm's"
        );
        assert!(
            !allowed_verb(&v("questions answer q-1 --yes fine")),
            "-y is the global auto-approval, which widens a resumed run"
        );
        assert!(!allowed_verb(&v("outbox reject ob-1 -y")));
        assert!(
            !allowed_verb(&v("questions answer q-1 --tool fs_read fine")),
            "the real flag"
        );
        assert!(!allowed_verb(&v("questions answer q-1 --skill s fine")));
        assert!(
            !allowed_verb(&v("questions answer q-1 -e high fine")),
            "the short effort"
        );
        assert!(
            !allowed_verb(&v("tasks set t-1 -w/tmp/x")),
            "an attached short value"
        );
        assert!(!allowed_verb(&v("tasks set t-1 -mx")));
        assert!(
            !allowed_verb(&v("tasks set t-1 -vw /tmp/x")),
            "a stacked short group"
        );
        assert!(!allowed_verb(&v("questions answer q-1 -vy fine")));
        assert!(
            allowed_verb(&v("tasks set t-1 --status done -v")),
            "verbose alone is fine"
        );
        assert!(!allowed_verb(&v("tasks set t-1 --no-mcp-server graph")));
        assert!(
            !allowed_verb(&v("questions answer q-1 --max-cost 999 fine")),
            "a ceiling the resumed run would honour"
        );
        // The answer's shape, strict.
        let out: PrincipalOutput = serde_json::from_str(
            r#"{"acts":[{"verb":["outbox","reject","ob-1","--reason","wrong date"],"reason":"gold: the date is not the meeting's"}],"deny":[{"tool":"shell","input_contains":"rm -rf","reason":"no"}]}"#,
        )
        .unwrap();
        assert_eq!(out.acts.len(), 1);
        assert_eq!(out.deny[0].tool, "shell");
        assert!(
            serde_json::from_str::<PrincipalOutput>(r#"{"acts":[],"sessions":[]}"#).is_err(),
            "an unknown key is the principal's error"
        );
        // The denials file.
        let home = std::env::temp_dir().join(format!("mecha-exp-deny-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        write_denials(&home, &out.deny).unwrap();
        let text = std::fs::read_to_string(denials_file(&home)).unwrap();
        assert!(
            text.contains("[[deny]]") && text.contains("rm -rf"),
            "{text}"
        );
        write_denials(&home, &[]).unwrap();
        assert!(
            !denials_file(&home).exists(),
            "an empty list clears the file"
        );
        let _ = std::fs::remove_dir_all(&home);
        // A failed before_task is not stood for by the after_task's success
        // at the same position: the point is part of a line's identity.
        let line = |attempt: u32, point: PrincipalPoint, status: StageStatus| StageRun {
            lifetime: "full__r1".into(),
            arm: "full".into(),
            stage: StageLever::Principal,
            after_position: 3,
            attempt,
            started_at: "t0".into(),
            finished_at: "t1".into(),
            status,
            exit_code: None,
            error: None,
            point: Some(point),
            acts: Vec::new(),
            refusals: Vec::new(),
        };
        let ledger = vec![
            line(1, PrincipalPoint::BeforeTask, StageStatus::Failed),
            line(2, PrincipalPoint::AfterTask, StageStatus::Running),
            line(2, PrincipalPoint::AfterTask, StageStatus::Done),
        ];
        let h = stage_health(&ledger);
        assert_eq!((h.done, h.failed, h.broken()), (1, 1, 1), "{h:?}");
        let ledger = vec![
            line(1, PrincipalPoint::BeforeTask, StageStatus::Running),
            line(2, PrincipalPoint::AfterTask, StageStatus::Done),
            line(3, PrincipalPoint::BeforeTask, StageStatus::Done),
        ];
        let h = stage_health(&ledger);
        assert_eq!(
            (h.done, h.interrupted),
            (2, 0),
            "an interrupted before_task is stood for only by a later before_task: {h:?}"
        );
        // A refusal's firings are counted off the session's own sentence.
        let rules = vec![
            crate::tool::DenialRule {
                tool: "shell".into(),
                input_contains: Some("echo".into()),
                reason: "use printf".into(),
            },
            crate::tool::DenialRule {
                tool: "fs_write".into(),
                input_contains: None,
                reason: "read only today".into(),
            },
        ];
        let session = "{\"content\":\"Denied by the user: use printf\"}\n{\"x\":1}\n{\"content\":\"Denied by the user: use printf, please\"}\n";
        let out = refusal_outcomes(&rules, Some(session));
        assert_eq!((out[0].fired, out[1].fired), (Some(2), Some(0)), "{out:?}");
        let unknown = refusal_outcomes(&rules, None);
        assert!(
            unknown.iter().all(|r| r.fired.is_none()),
            "no session to read is unknown, never zero: {unknown:?}"
        );
        assert_eq!(out[1].tool, "fs_write");
        // A reason with a quote is escaped in the file and still counts.
        let quoted = vec![crate::tool::DenialRule {
            tool: "shell".into(),
            input_contains: None,
            reason: "use \"printf\" here".into(),
        }];
        let session = "{\"content\":\"Denied by the user: use \\\"printf\\\" here\"}\n";
        assert_eq!(
            refusal_outcomes(&quoted, Some(session))[0].fired,
            Some(1),
            "{session}"
        );
        write_denials(&home, &rules).unwrap();
        assert_eq!(read_denials(&home).unwrap(), rules);
        let _ = std::fs::remove_dir_all(&home);
        // The ledger's stage for it is not a lever.
        assert!(!StageLever::ALL.contains(&StageLever::Principal));
        assert_eq!(
            StageLever::parse("principal"),
            None,
            "never named in a manifest"
        );
        assert_eq!(StageLever::Principal.argv(), None);
        let wire: StageLever = serde_json::from_str("\"principal\"").unwrap();
        assert_eq!(wire, StageLever::Principal);
        assert_eq!(PrincipalPoint::AfterTask.as_str(), "after_task");
        let later: PrincipalPoint = serde_json::from_str("\"mid_task\"").unwrap();
        assert_eq!(
            later,
            PrincipalPoint::Unknown,
            "a point a later build names"
        );
        assert!(
            serde_json::from_str::<PrincipalRequest>(
                r#"{"verb":["outbox","approve","x","-y"],"exit_code":0,"ok":true}"#,
            )
            .is_err(),
            "an exit code is the driver's alone; a principal that claims one is off the contract"
        );
        let plain: PrincipalAct =
            serde_json::from_str::<PrincipalRequest>(r#"{"verb":["outbox","approve","x","-y"]}"#)
                .unwrap()
                .into();
        assert_eq!((plain.exit_code, plain.ok), (None, None));
        // The ledger's line, with the driver's fields filled, reads back:
        // the one type that refused them made every line with an act torn.
        let dir = std::env::temp_dir().join(format!("mecha-exp-acts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = ExperimentStore::open(&dir, "life").unwrap();
        let mut filled = plain.clone();
        filled.exit_code = Some(1);
        filled.ok = Some(false);
        let line = StageRun {
            lifetime: "full__r1".into(),
            arm: "full".into(),
            stage: StageLever::Principal,
            after_position: 2,
            attempt: 1,
            started_at: "t0".into(),
            finished_at: "t1".into(),
            status: StageStatus::Failed,
            exit_code: None,
            error: Some("`mecha outbox approve x -y` exited 1".into()),
            point: Some(PrincipalPoint::AfterTask),
            acts: vec![filled.clone()],
            refusals: Vec::new(),
        };
        store.record_stage(&line).unwrap();
        let (runs, torn) = store.stage_runs("full__r1").unwrap();
        assert_eq!((runs.len(), torn), (1, 0), "a filled act round-trips");
        assert_eq!(runs[0].acts, vec![filled]);
        assert_eq!(stage_health(&runs).failed, 1, "and the hold sees it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a stage accepted inside the home reaches the next task: the
    /// home's `overrides.toml` folds into the rendered config over the
    /// arm's rendering, and the arm's own pinned keys still win.
    #[test]
    fn the_homes_accepted_overrides_reach_the_next_task_and_the_arms_pins_win() {
        let home = std::env::temp_dir().join(format!("mecha-exp-fold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let overrides = home.join(HOME_OVERRIDES);
        std::fs::create_dir_all(overrides.parent().unwrap()).unwrap();
        std::fs::write(
            &overrides,
            "[[override]]\nkey = \"max_turns\"\nvalue = \"30\"\ncandidate = \"c1\"\naccepted_at = \"2026-09-04T00:00:00Z\"\n\
             [[override]]\nkey = \"compact_at_tokens\"\nvalue = \"24000\"\ncandidate = \"c2\"\naccepted_at = \"2026-09-04T00:00:00Z\"\n",
        )
        .unwrap();
        let real = crate::config::Config::default();
        let arm = Arm {
            overrides: vec!["max_turns=20".into()],
            ..Arm::default()
        };
        let mut config = child_invocation(&real, &arm, None).unwrap().config;
        assert_eq!(config.agent.max_turns, 20, "the arm's rendering");
        let moved = fold_home_overrides(&mut config, &home, &arm).unwrap();
        assert_eq!(
            moved,
            vec!["max_turns".to_string(), "compact_at_tokens".to_string()],
            "the keys the home's loop moved, for the driver to treat as pins"
        );
        assert_eq!(config.agent.max_turns, 20, "the arm pins the treatment key");
        assert_eq!(
            config.agent.compact_at_tokens,
            Some(24000),
            "what ruminate accepted in this home reaches the next task"
        );
        let mut plain = child_invocation(&real, &Arm::default(), None)
            .unwrap()
            .config;
        fold_home_overrides(&mut plain, &home, &Arm::default()).unwrap();
        assert_eq!(plain.agent.max_turns, 30, "an unpinned key moves");
        // The machine's own overrides never ride in: seeding drops the copy,
        // and a key the operator pinned in the file keeps the file's value.
        let fake_real = std::env::temp_dir().join(format!("mecha-exp-real-{}", std::process::id()));
        let fresh = std::env::temp_dir().join(format!("mecha-exp-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&fake_real);
        let _ = std::fs::remove_dir_all(&fresh);
        std::fs::create_dir_all(fake_real.join("learning/harness")).unwrap();
        std::fs::copy(&overrides, fake_real.join(HOME_OVERRIDES)).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        seed_home(&fake_real, &fresh).unwrap();
        assert!(
            fresh.join("learning/harness").is_dir(),
            "the harness store is seeded"
        );
        assert!(
            !fresh.join(HOME_OVERRIDES).exists(),
            "but not the machine's overrides"
        );
        let mut pinned = crate::config::Config::default();
        pinned.agent.max_turns = 40; // what the operator's config.toml said
        let mut cfg = child_invocation(&pinned, &Arm::default(), None)
            .unwrap()
            .config;
        fold_home_overrides(&mut cfg, &fresh, &Arm::default()).unwrap();
        assert_eq!(
            cfg.agent.max_turns, 40,
            "the file's pin survives a fresh home"
        );
        let _ = std::fs::remove_dir_all(&fake_real);
        let _ = std::fs::remove_dir_all(&fresh);
        // A torn file is the task's error, never a silent nothing.
        std::fs::write(&overrides, "[[override]]\nkey = \"max_turns\"\n").unwrap();
        let e = format!(
            "{:#}",
            fold_home_overrides(&mut plain, &home, &Arm::default()).unwrap_err()
        );
        assert!(e.contains("could not be read"), "{e}");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The one stage lever that is a switch rides into the trial home's
    /// config, and only when the arm names it.
    #[test]
    fn sensors_in_brief_off_rides_in_the_childs_config() {
        let real = crate::config::Config::default();
        let mut arm = Arm::default();
        assert!(
            child_invocation(&real, &arm, None)
                .unwrap()
                .config
                .agent
                .sensors_in_brief
        );
        arm.stages_off = vec!["sensors_in_brief".into()];
        let child = child_invocation(&real, &arm, None).unwrap();
        assert!(!child.config.agent.sensors_in_brief);
        assert!(child.flags.is_empty(), "a switch, not a flag");
    }

    /// The gate over arm sets: each treatment arm paired with the control by
    /// episode, the task outcome entering as a cost, the holdout drawn by the
    /// manifest's seed, and a pair whose trial cannot answer the metric
    /// dropped rather than scored.
    #[test]
    fn the_judge_pairs_each_treatment_arm_against_the_control() {
        let m = Manifest::parse(MANIFEST).unwrap();
        let mut trials = Vec::new();
        for task in 0..12 {
            let task = format!("t{task}");
            for seed in [1, 2] {
                trials.push(done("full", &task, seed, true, 10));
                // bare fails everything: a clear loss on `failure`.
                trials.push(done("bare", &task, seed, false, 10));
                // quiet uses fewer turns: a win on `turns`.
                trials.push(done("quiet", &task, seed, true, 6));
            }
        }
        // One quiet trial never got stats: its pair drops.
        trials.iter_mut().find(|t| t.arm == "quiet").unwrap().stats = None;
        let verdicts = judge(&m, &trials, &[], 0);
        assert_eq!(verdicts.len(), 2);
        let bare = verdicts.iter().find(|v| v.arm == "bare").unwrap();
        assert_eq!(bare.metric, ExpMetric::Failure);
        assert_eq!(bare.pairs, 24);
        assert_eq!(bare.selection + bare.holdout, 24);
        assert!(bare.holdout >= crate::candidate::MIN_HOLDOUT_PAIRS);
        assert!(
            matches!(
                bare.judgement.disposition,
                crate::candidate::Disposition::Reject(_)
            ),
            "{:?}",
            bare.judgement.disposition
        );
        let quiet = verdicts.iter().find(|v| v.arm == "quiet").unwrap();
        assert_eq!(quiet.pairs, 23, "the statless trial dropped its pair");
        assert!(
            !matches!(
                quiet.judgement.disposition,
                crate::candidate::Disposition::Reject(_)
            ),
            "{:?}",
            quiet.judgement.disposition
        );
        // Deterministic: the same manifest draws the same holdout.
        let again = judge(&m, &trials, &[], 0);
        assert_eq!(again[0].holdout, verdicts[0].holdout);
        assert_eq!(
            again[0].judgement.holdout.wins,
            verdicts[0].judgement.holdout.wins
        );
    }

    #[test]
    fn the_store_keeps_its_rows_over_the_design_and_reads_a_torn_row_as_a_finding() {
        let root = std::env::temp_dir().join(format!("mecha-exp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = ExperimentStore::open(&root, "levers").unwrap();
        let m = store.create(MANIFEST).unwrap();
        assert!(store.create(MANIFEST).is_err(), "written once");
        let tasks = vec!["a".to_string()];
        let (planned, skipped) = store.plan(&m, &tasks, "local", "m").unwrap();
        assert_eq!(planned.len(), 6);
        assert_eq!(skipped, 0);
        let mut first = planned[0].clone();
        first.status = TrialStatus::Done;
        first.passed = Some(true);
        store.save_trial(&first).unwrap();
        std::fs::write(store.trial_path("torn"), b"{not json").unwrap();
        let (planned, skipped) = store.plan(&m, &tasks, "local", "m").unwrap();
        assert_eq!(planned[0].status, TrialStatus::Done, "the store's row wins");
        assert_eq!(skipped, 1, "a torn row is counted, not read as pending");
        // An unknown status reads as unknown, never as a failed file.
        let raw = serde_json::to_string(&first)
            .unwrap()
            .replace("\"done\"", "\"vanished\"");
        let t: Trial = serde_json::from_str(&raw).unwrap();
        assert_eq!(t.status, TrialStatus::Unknown);
        let home = store.arm_home("bare").unwrap();
        assert!(is_experiment_home(&home));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_experiment_ref_round_trips_and_a_bad_one_is_none() {
        let r = ExperimentRef {
            exp_id: "levers".into(),
            trial_id: "bare__a__r1".into(),
            arm: "bare".into(),
            actor: "bare__a__r1".into(),
            role: None,
            task: "a".into(),
            repetition: 1,
            condition_hash: "h".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<ExperimentRef>(&json).unwrap(), r);
        assert!(serde_json::from_str::<ExperimentRef>("{\"exp_id\":1}").is_err());
    }

    const FIXTURED: &str = r#"
name = "home"
kind = "lifetime"
split_seed = 3
[tasks]
cases = "eval/home-cases.jsonl"
fixture = "eval/workspace"
ids = ["a"]
[fixtures]
charter = "eval/fixtures/home/charter.toml"
outbox_tools = ["mail__mail_send"]
[[fixtures.mcp]]
name = "graph"
command = "python3"
args = ["eval/fixtures/board_server.py"]
prefix_tools = false
seed = "eval/fixtures/home/board"
[fixtures.mcp.capabilities]
untrusted_input = true
[[fixtures.mcp]]
name = "mail"
command = "python3"
args = ["eval/fixtures/mail_server.py"]
[arms.full]
preset = "full"
"#;

    /// A manifest names fixture servers; the names are unique, carry no
    /// `__`, name a command — and a fixture charter needs a fixture world.
    #[test]
    fn fixtures_are_named_on_the_manifest_and_validated_at_load() {
        let m = Manifest::parse(FIXTURED).unwrap();
        assert_eq!(
            m.fixtures.names(),
            vec!["graph".to_string(), "mail".to_string()]
        );
        assert!(!m.fixtures.is_empty());
        assert_eq!(m.fixtures.mcp[0].prefix_tools, Some(false));
        assert!(m.fixtures.mcp[0].capabilities.untrusted_input);
        assert_eq!(
            m.fixtures.mcp[0].seed.as_deref(),
            Some(Path::new("eval/fixtures/home/board"))
        );
        // A `single` may carry fixtures too: eval's graph cases are the case.
        let single = FIXTURED
            .replace("kind = \"lifetime\"\n", "")
            .replace("ids = [\"a\"]\n", "");
        assert!(Manifest::parse(&single).is_ok());
        let twice = FIXTURED.replace("name = \"mail\"", "name = \"graph\"");
        assert!(Manifest::parse(&twice)
            .unwrap_err()
            .to_string()
            .contains("named twice"));
        let sep = FIXTURED.replace("name = \"mail\"", "name = \"my__mail\"");
        assert!(Manifest::parse(&sep)
            .unwrap_err()
            .to_string()
            .contains("__"));
        let nocmd = FIXTURED.replace(
            "command = \"python3\"\nargs = [\"eval/fixtures/mail_server.py\"]",
            "command = \"\"",
        );
        assert!(Manifest::parse(&nocmd)
            .unwrap_err()
            .to_string()
            .contains("no command"));
        let charter_alone = "name = \"x\"\nsplit_seed = 1\n[tasks]\ncases = \"c\"\nfixture = \"f\"\n[fixtures]\ncharter = \"ch.toml\"\n[arms.a]\n";
        assert!(
            Manifest::parse(charter_alone)
                .unwrap_err()
                .to_string()
                .contains("without a fixture server"),
            "a fixture charter over the operator's live servers is refused"
        );
        // The route is spelled, never inherited (found on review): absent
        // is refused, `[]` is a decision, and every name is a prefixed
        // fixture's tool.
        let route = "outbox_tools = [\"mail__mail_send\"]";
        let unrouted = FIXTURED.replace(&format!("{route}\n"), "");
        let err = Manifest::parse(&unrouted).unwrap_err().to_string();
        assert!(err.contains("no `outbox_tools`"), "{err}");
        let none = FIXTURED.replace(route, "outbox_tools = []");
        assert!(Manifest::parse(&none).unwrap().fixtures.routed().is_empty());
        let board = FIXTURED.replace(route, "outbox_tools = [\"kg_task_update\"]");
        let err = Manifest::parse(&board).unwrap_err().to_string();
        assert!(
            err.contains("not a prefixed fixture server's tool"),
            "an unprefixed server's tool: {err}"
        );
        let live = FIXTURED.replace(route, "outbox_tools = [\"docs__docs_create\"]");
        assert!(Manifest::parse(&live).is_err(), "a live server's tool");
        let bare = FIXTURED.replace(route, "outbox_tools = [\"mail__\"]");
        assert!(Manifest::parse(&bare).is_err(), "a prefix alone");
        let route_alone = "name = \"x\"\nsplit_seed = 1\n[tasks]\ncases = \"c\"\nfixture = \"f\"\n[fixtures]\noutbox_tools = []\n[arms.a]\n";
        assert!(Manifest::parse(route_alone)
            .unwrap_err()
            .to_string()
            .contains("without a fixture server"));
        // Absent: the default, empty, and the manifest from before the field parses.
        let m = Manifest::parse(MANIFEST).unwrap();
        assert!(m.fixtures.is_empty());
        assert!(m.fixtures.names().is_empty());
    }

    /// Rendering puts exactly the fixtures into `[[mcp]]`, drops every live
    /// server, hands each its store under the home, seeds it once, and
    /// resolves the manifest's relative file paths against the base.
    #[test]
    fn fixtures_replace_the_operators_servers_and_persist_under_the_home() {
        let base = std::env::temp_dir().join(format!("mecha-fx-base-{}", uuid::Uuid::new_v4()));
        let home = std::env::temp_dir().join(format!("mecha-fx-home-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("srv")).unwrap();
        std::fs::create_dir_all(base.join("seed")).unwrap();
        std::fs::write(base.join("srv/board.py"), b"# a server").unwrap();
        std::fs::write(base.join("seed/board.json"), b"{\"tasks\":{}}").unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let text = r#"
name = "fx"
split_seed = 1
[tasks]
cases = "c"
fixture = "f"
[fixtures]
outbox_tools = ["graph__kg_upsert"]
[[fixtures.mcp]]
name = "graph"
command = "python3"
args = ["srv/board.py", "--flag", "not/a/file.py"]
seed = "seed"
[arms.a]
"#;
        let m = Manifest::parse(text).unwrap();
        let mut config = crate::config::Config::default();
        config.mcp.push(crate::config::McpServerConfig {
            name: "mail".into(),
            command: "/usr/bin/mecha-mail".into(),
            ..Default::default()
        });
        config.outbox.tools = vec!["mail__mail_send".into(), "factory__bundle_publish".into()];
        config.outbox.publish_tools = vec!["factory__bundle_publish".into()];
        m.fixtures.apply(&mut config, &home, &base, false).unwrap();
        assert_eq!(config.mcp.len(), 1, "the operator's live server is gone");
        assert_eq!(
            config.outbox.tools,
            vec!["graph__kg_upsert".to_string()],
            "the route is the world's, not the operator's"
        );
        assert!(
            config.outbox.publish_tools.is_empty(),
            "no fixture publishes"
        );
        let srv = &config.mcp[0];
        assert_eq!(srv.name, "graph");
        assert!(
            Path::new(&srv.args[0]).is_absolute() && srv.args[0].ends_with("srv/board.py"),
            "a relative file is resolved: {}",
            srv.args[0]
        );
        assert_eq!(srv.args[1], "--flag", "a flag is left alone");
        assert_eq!(
            srv.command, "python3",
            "a bare command is a PATH lookup, never resolved"
        );
        std::fs::write(base.join("python3"), b"#!/bin/sh\n").unwrap();
        let mut argv = vec!["python3".to_string(), "srv/board.py".to_string()];
        resolve_file_args(&mut argv, &base);
        assert_eq!(argv[0], "python3", "even beside a file of that name");
        assert!(argv[1].ends_with("srv/board.py") && Path::new(&argv[1]).is_absolute());
        let mut argv = vec!["srv/board.py".to_string()];
        resolve_file_args(&mut argv, &base);
        assert!(
            Path::new(&argv[0]).is_absolute(),
            "a command spelled as a path is resolved"
        );
        assert_eq!(
            srv.args[2], "not/a/file.py",
            "a path that is not there is left alone"
        );
        assert_eq!(
            srv.env.get(FIXTURE_DIR_ENV).map(String::as_str),
            Some(fixture_dir(&home, "graph").to_string_lossy().as_ref())
        );
        assert!(srv.env_passthrough.is_empty() && !srv.sandbox && !srv.disabled);
        assert_eq!(
            std::fs::read(fixture_dir(&home, "graph").join("board.json")).unwrap(),
            b"{\"tasks\":{}}",
            "seeded"
        );
        // Seeded once: the second render leaves what the first lifetime did.
        std::fs::write(
            fixture_dir(&home, "graph").join("board.json"),
            b"{\"tasks\":{\"t\":1}}",
        )
        .unwrap();
        m.fixtures.apply(&mut config, &home, &base, false).unwrap();
        assert_eq!(
            std::fs::read(fixture_dir(&home, "graph").join("board.json")).unwrap(),
            b"{\"tasks\":{\"t\":1}}"
        );
        assert!(fixture_dir(&home, "graph").join(FIXTURE_SEEDED).exists());
        // A `single` starts every trial from the seed (found on review).
        m.fixtures.apply(&mut config, &home, &base, true).unwrap();
        assert_eq!(
            std::fs::read(fixture_dir(&home, "graph").join("board.json")).unwrap(),
            b"{\"tasks\":{}}",
            "fresh drops what the last trial did"
        );
        // A store without the marker is torn and is rebuilt.
        std::fs::remove_file(fixture_dir(&home, "graph").join(FIXTURE_SEEDED)).unwrap();
        std::fs::write(fixture_dir(&home, "graph").join("board.json"), b"torn").unwrap();
        m.fixtures.apply(&mut config, &home, &base, false).unwrap();
        assert_eq!(
            std::fs::read(fixture_dir(&home, "graph").join("board.json")).unwrap(),
            b"{\"tasks\":{}}"
        );
        // A missing seed is an error, never an empty board.
        let missing = text.replace("seed = \"seed\"", "seed = \"nowhere\"");
        let m2 = Manifest::parse(&missing).unwrap();
        let home2 = std::env::temp_dir().join(format!("mecha-fx-home-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home2).unwrap();
        assert!(m2
            .fixtures
            .apply(&mut config, &home2, &base, false)
            .unwrap_err()
            .to_string()
            .contains("not a directory"));
        // Fails closed: no store is left behind for the next render to
        // treat as seeded (found on review) — and once the seed is there,
        // the next render seeds.
        assert!(
            !fixture_dir(&home2, "graph").exists(),
            "a failed seed leaves nothing"
        );
        assert!(
            std::fs::read_dir(home2.join("fixtures"))
                .map(|d| d.count() == 0)
                .unwrap_or(true),
            "no staging directory is left either"
        );
        std::fs::create_dir_all(base.join("nowhere")).unwrap();
        std::fs::write(base.join("nowhere/board.json"), b"{}").unwrap();
        m2.fixtures
            .apply(&mut config, &home2, &base, false)
            .unwrap();
        assert_eq!(
            std::fs::read(fixture_dir(&home2, "graph").join("board.json")).unwrap(),
            b"{}"
        );
        // No fixtures: the config is untouched, route included.
        let mut live = crate::config::Config::default();
        live.mcp.push(crate::config::McpServerConfig {
            name: "mail".into(),
            ..Default::default()
        });
        live.outbox.tools = vec!["mail__mail_send".into()];
        Manifest::parse(MANIFEST)
            .unwrap()
            .fixtures
            .apply(&mut live, &home, &base, false)
            .unwrap();
        assert_eq!(live.mcp.len(), 1);
        assert_eq!(live.outbox.tools.len(), 1);
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&home2);
    }

    /// The fixture charter is written over the home's, parsed first, and
    /// only when the manifest names one.
    #[test]
    fn the_fixture_charter_is_written_over_the_seeded_one() {
        let base = std::env::temp_dir().join(format!("mecha-fx-base-{}", uuid::Uuid::new_v4()));
        let home = std::env::temp_dir().join(format!("mecha-fx-home-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("charter.toml"), b"# the operator's\n").unwrap();
        std::fs::write(
            base.join("ch.toml"),
            b"[[line]]\nid = \"answer\"\ntext = \"Answer what waits on me.\"\n",
        )
        .unwrap();
        let mut fx = Fixtures::default();
        fx.apply_charter(&home, &base).unwrap();
        assert_eq!(
            std::fs::read(home.join("charter.toml")).unwrap(),
            b"# the operator's\n",
            "none named: untouched"
        );
        fx.charter = Some("ch.toml".into());
        fx.apply_charter(&home, &base).unwrap();
        assert!(
            String::from_utf8(std::fs::read(home.join("charter.toml")).unwrap())
                .unwrap()
                .contains("Answer what waits")
        );
        std::fs::write(base.join("ch.toml"), b"[[line]]\nnot = toml =\n").unwrap();
        assert!(fx
            .apply_charter(&home, &base)
            .unwrap_err()
            .to_string()
            .contains("does not parse"));
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Release and closure reach a server: permitted only under a manifest
    /// that names fixtures; every other verb as before, and the shape rules
    /// hold for all of them.
    #[test]
    fn a_verb_that_reaches_a_server_needs_fixture_servers() {
        let v = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert!(
            allowed_verb(&v("outbox approve ob-1")),
            "the shape is an owner's verb now"
        );
        assert!(reaches_a_server(&v("outbox approve ob-1")));
        assert!(reaches_a_server(&v("tasks set t-1 --status done")));
        assert!(!reaches_a_server(&v("outbox reject ob-1 --reason no")));
        assert!(!reaches_a_server(&v("tasks steer t-1 go left")));
        assert!(!reaches_a_server(&v(
            "questions answer q-1 --unattended yes"
        )));
        // Without fixtures: refused, naming the reason.
        let err = permitted_verb(&v("outbox approve ob-1"), false).unwrap_err();
        assert!(err.contains("names no fixture servers"), "{err}");
        let err = permitted_verb(&v("tasks set t-1 --status done"), false).unwrap_err();
        assert!(err.contains("reaches a server"), "{err}");
        assert!(permitted_verb(&v("outbox reject ob-1 --reason no"), false).is_ok());
        // With fixtures: permitted; the shape rules still apply.
        assert!(permitted_verb(&v("outbox approve ob-1"), true).is_ok());
        assert!(permitted_verb(&v("tasks set t-1 --session s-1 --status done"), true).is_ok());
        assert!(
            permitted_verb(&v("outbox approve ob-1 --yes"), true).is_err(),
            "-y/--yes is the driver's word, never the principal's"
        );
        assert!(permitted_verb(&v("outbox approve ob-1 --workspace /tmp"), true).is_err());
        assert!(permitted_verb(&v("run do it"), true)
            .unwrap_err()
            .contains("not an owner's verb"));
    }

    /// A release lands where the draft's tool does: a fixture server's tool
    /// by prefix, and nothing else.
    #[test]
    fn a_release_target_is_a_fixture_servers_tool_by_prefix() {
        let fx = vec!["graph".to_string(), "mail".to_string()];
        assert!(release_target_is_fixture("mail__mail_send", &fx));
        assert!(release_target_is_fixture(
            "mail__calendar_create_event",
            &fx
        ));
        assert!(
            !release_target_is_fixture("docs__docs_create", &fx),
            "a live server's"
        );
        assert!(
            !release_target_is_fixture("http_fetch", &fx),
            "a builtin sink"
        );
        assert!(
            !release_target_is_fixture("kg_task_update", &fx),
            "an unprefixed server's tool cannot be attributed"
        );
        assert!(
            !release_target_is_fixture("mail__", &fx),
            "a prefix alone names no tool"
        );
        assert!(
            !release_target_is_fixture("mail__mail_send", &[]),
            "an arm with MCP off reaches none"
        );
    }

    /// The hash carries the fixture names only when a manifest names any,
    /// so every hash minted before fixtures existed keeps its value.
    #[test]
    fn the_condition_hash_names_the_fixtures_only_when_there_are_any() {
        let plain = condition_hash_with_stages(&[], &[], "p", "m", Some(1), &[]);
        assert_eq!(
            plain,
            condition_hash_of(&[], &[], "p", "m", Some(1), &[], &[])
        );
        let fixtured = condition_hash_of(
            &[],
            &[],
            "p",
            "m",
            Some(1),
            &[],
            &["mail".into(), "graph".into()],
        );
        assert_ne!(plain, fixtured);
        assert_eq!(
            fixtured,
            condition_hash_of(
                &[],
                &[],
                "p",
                "m",
                Some(1),
                &[],
                &["graph".into(), "mail".into()]
            ),
            "order-free"
        );
        let m = Manifest::parse(FIXTURED).unwrap();
        let rows = m.trials(&["a".into()], "p", "m");
        assert_eq!(
            rows[0].condition_hash,
            condition_hash_of(
                &[],
                &[],
                "p",
                "m",
                None,
                &[],
                &["graph".into(), "mail".into()]
            )
        );
    }

    /// The principal is told which fixtures its verbs can reach; an old
    /// input without the field reads as none.
    #[test]
    fn the_principal_input_names_the_fixtures_it_can_reach() {
        let m = Manifest::parse(FIXTURED).unwrap();
        let case: crate::eval::EvalCase =
            serde_json::from_str(r#"{"id":"a","prompt":"p"}"#).unwrap();
        let input = PrincipalInput {
            point: PrincipalPoint::AfterTask,
            experiment: m.name.clone(),
            lifetime: "full__r1".into(),
            arm: "full".into(),
            position: 0,
            home: "/h".into(),
            workspace: "/w".into(),
            case,
            trial: None,
            pending_outbox: vec![],
            open_questions: vec![],
            fixtures: m.fixtures.names(),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["fixtures"], serde_json::json!(["graph", "mail"]));
        let mut old = json.clone();
        old.as_object_mut().unwrap().remove("fixtures");
        let back: PrincipalInput = serde_json::from_value(old).unwrap();
        assert!(back.fixtures.is_empty());
    }
}
