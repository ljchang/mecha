//! `mecha rules` — the lifecycle a rule enters *after* acceptance.
//!
//! Additions have a gate (`mecha learn --propose` → `mecha proposals`);
//! this is tenure. `list` folds the validation ledger into per-rule tallies
//! and surfaces the pressure — attributed regressions, never-validated
//! rules, age. `retire` and `restore` are the human acting directly, the
//! apply-with-git-undo path, same standing as a direct `mecha learn`.
//! `propose-retirements` is the unattended path: a deterministic scan of the
//! ledger — no model anywhere — that stages an `enabled = false` +
//! `retired_*` diff through the same proposal gate every other rule change
//! passes. Retirement is a flag, never a deletion: the rule stays in the
//! file as evidence, the learner is told it was measured harmful, and
//! `restore` can undo what erasure could not.
//!
//! The threshold is deliberately conservative (default 3 attributed
//! regressions) and counts only *attributed* regressions — bisection
//! verdicts, not block-level context — because the library-drift result
//! cuts both ways: unpruned stores go negative, and over-eager retirement
//! measurably hurt too. No decay, no TTL, no usage-based eviction: low
//! usage is a review signal, only measured harm argues for retirement, and
//! a human accepts the argument.
//!
//! **Tenure by the owner's verdicts sits beside that** (row 2e-5b, R41;
//! `mecha_core::tenure`): the Wilson lower bound of the owner-accept rate on
//! the runs that carried a rule releases it from probation and marks it
//! tenured; it retires, narrows and evicts nothing. And a rule whose region
//! has gone quiet is *reported* (row 2e-5c) and keeps loading.

use anyhow::{bail, Context, Result};
use mecha_core::learning::{
    judge_convicted, retire_threshold_for, rule_tallies, tally_for, LeapRun, LearningStore,
    Proposal, Rule, RuleTally, ValidationRecord, Verdict,
};
use mecha_core::replay_priority::Recurrence;
use mecha_core::session::{Session, SessionKind};
use mecha_core::situation::{BoardStatuses, GoalKey, GoalLiveness, TriggerState};
use mecha_core::tenure::{Quiet, Tally};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Every rule with its ledger tallies and staleness (default).
    List {
        /// Machine-readable, for `/learning`.
        #[arg(long)]
        json: bool,
        /// Do not read the board: every `task:` goal is then unknown, never
        /// open. For the TUI and the web settings page, which shell out to
        /// this verb under budgets an MCP start cannot fit (the TUI blocks
        /// its event loop on it; the web gives it ten seconds). Trigger
        /// goals are still read, in place. The same budgets skip the
        /// session walks behind each rule's owner tenure and whether its
        /// region has gone quiet, which then read as not read on this path.
        #[arg(long)]
        no_board: bool,
    },
    /// Retire a rule by id (or unique prefix): kept in the file as evidence,
    /// never rendered into a prompt again.
    Retire {
        id: String,
        /// Why — recorded on the rule and shown to the learner so the same
        /// lesson does not come back under new wording.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Un-retire a rule by id (or unique prefix).
    Restore { id: String },
    /// One rule in full: its text, domain, state and ledger tally. What the
    /// TUI's `Enter` on the Rules pane runs — a rule is the one record here
    /// that rides in every future prompt, so it is the one most worth
    /// reading in full.
    Show {
        id: String,
        /// As on `list`: skip the board read. The TUI's `Enter` passes it.
        #[arg(long)]
        no_board: bool,
    },
    /// Scan the ledger and stage retirement proposals for rules the
    /// bisection keeps convicting. Deterministic; review with `mecha
    /// proposals`.
    ProposeRetirements {
        /// Apply the retirements directly instead of staging a proposal.
        ///
        /// Safe to automate in a way that promotion is not: this scan is a
        /// deterministic fold over the validation ledger with no model in it,
        /// it only ever *disables* rules, and a retired rule stays in the file
        /// as evidence. It is also the precondition for ungated learning —
        /// promotion without a working NoGo path is a ratchet.
        #[arg(long)]
        apply: bool,
        /// Attributed regressions required before a rule is proposed for
        /// retirement.
        #[arg(long, default_value_t = mecha_core::learning::DEFAULT_RETIRE_AT)]
        min_attributed: u32,
    },
}

pub async fn execute(global: &crate::GlobalOpts, args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    match args.cmd.unwrap_or(Cmd::List {
        json: false,
        no_board: false,
    }) {
        Cmd::List { json, no_board } => {
            let everything = all_rules(&store);
            let goals = goals_of(global, &everything, !no_board).await;
            let standing = (!no_board).then(|| Standing::read(&everything, chrono::Utc::now()));
            list(&store, json, &goals, standing.as_ref())
        }
        Cmd::Retire { id, reason } => retire(&store, &id, reason),
        Cmd::Restore { id } => restore(&store, &id),
        Cmd::Show { id, no_board } => {
            let (_, rules, i) = find_rule(&store, &id)?;
            let goals = goals_of(global, &[rules[i].clone()], !no_board).await;
            let standing =
                (!no_board).then(|| Standing::read(&[rules[i].clone()], chrono::Utc::now()));
            show(&store, &id, &goals, standing.as_ref())
        }
        Cmd::ProposeRetirements {
            min_attributed,
            apply,
        } => {
            // Only a probationary rule's leash can move on the owner's
            // record, so only those rules' sessions are read.
            let on_probation: std::collections::BTreeSet<String> = all_rules(&store)
                .iter()
                .filter(|r| r.active() && r.probation)
                .filter_map(|r| r.id.clone())
                .collect();
            let owner = owner_tally(&on_probation);
            for c in &owner.caveats {
                println!("owner tenure: {c}");
            }
            if let Some(why) = &owner.store_unreadable {
                println!("owner tenure: {why}; no rule is released on it this pass");
            }
            propose(&store, min_attributed, apply, Some(&owner))
        }
    }
}

/// The workspace/surface/goal keys some run's rules block was matched
/// against, off every run record of every transcript in the session store —
/// what a scope's workspace, surface or goal key must be found in to load
/// anywhere. `None` when the store cannot be read *in full*: a transcript
/// whose header does not parse (`Session::list_counting` counts them;
/// `list` drops them silently) or with a torn line before its first run
/// record (`Session::run_configs_streaming` refuses it) is evidence that could
/// not be read, never evidence of absence — one torn file must not print
/// "nowhere" about a rule that loads fine (found on review), and so is a
/// listing with no record carrying any of the keys — a missing store, or one
/// written before the fields existed. Every attach's record counts, since
/// a rule can be minted from a resumed run's keys (found on review); read
/// streaming rather than slurped, once per roster, and only when a scope
/// names one of those keys (`needs_presented_keys`) — the TUI's Rules
/// pane runs the roster on a keypress.
type Presented = Vec<Keys>;

/// One run record's matched keys: `(rules_workspace, rules_surface,
/// rules_goal)`.
type Keys = (Option<PathBuf>, Option<SessionKind>, Option<GoalKey>);

/// Whether a rule's answer consults the store at all: active, with no
/// parked or corpus-mark surface and no parked goal (those are answered
/// without it), and a workspace, surface or goal on its scope. One predicate for both gates —
/// whether to walk, and which pairs to walk for — because the early exit
/// in `presented_keys_in` is sound only if every rule that will read the
/// keys has its pair in `wanted`. Built from every rule, a retired rule
/// scoped to a workspace that went dark kept the exit from firing, and
/// the full walk it forced could meet a torn transcript and turn every
/// live rule's answer into unknown on the strength of one dead rule
/// (found on review).
fn needs_keys(r: &Rule) -> bool {
    r.active()
        && r.scope.as_ref().is_some_and(|s| {
            s.surface_unread.is_none()
                && !matches!(s.goal, Some(GoalKey::Unread(_)))
                && !s
                    .surface
                    .is_some_and(|k| mecha_core::situation::Situation::MARK_KINDS.contains(&k))
        })
        && r.scope
            .as_ref()
            .map(|s| s.scope())
            .is_some_and(|s| s.workspace.is_some() || s.surface.is_some() || s.goal.is_some())
}

fn needs_presented_keys(rules: &[&Rule]) -> bool {
    rules.iter().any(|r| needs_keys(r))
}

fn presented_keys(rules: &[&Rule]) -> Option<Presented> {
    presented_keys_from(rules, &Session::default_dir().ok()?)
}

fn presented_keys_from(rules: &[&Rule], dir: &Path) -> Option<Presented> {
    if !needs_presented_keys(rules) {
        return None;
    }
    let wanted: Presented = rules
        .iter()
        .filter(|r| needs_keys(r))
        .filter_map(|r| r.scope.as_ref().map(|s| s.scope()))
        .map(|s| (s.workspace, s.surface, s.goal))
        .collect();
    presented_keys_in(dir, &wanted)
}

/// Whether one run record's keys present one scope's keys: each key the
/// scope names must be the record's — a goal by name, since a parked goal
/// is presented by nothing (`Situation::matches`).
fn pair_presents(record: &Keys, scope: &Keys) -> bool {
    scope
        .0
        .as_ref()
        .is_none_or(|sw| record.0.as_ref() == Some(sw))
        && scope.1.is_none_or(|sk| record.1 == Some(sk))
        && match &scope.2 {
            None => true,
            Some(GoalKey::Named(g)) => record.2.as_ref().and_then(GoalKey::named) == Some(g),
            Some(GoalKey::Unread(_)) => false,
        }
}

/// `wanted` is the pairs the scoped rules name: the walk stops at the
/// first transcripts that present them all — newest first, so a rule
/// scoped to the workspace in use is answered by one file rather than the
/// whole store, which the TUI's reload reads on a keypress (found on
/// review). An early exit happens only on a positive answer for every
/// key; the unknown arms below are reached only by a walk that ran out.
fn presented_keys_in(dir: &Path, wanted: &[Keys]) -> Option<Presented> {
    let (listed, unreadable) = Session::list_counting(dir).ok()?;
    if unreadable > 0 {
        return None;
    }
    let mut out = Presented::new();
    for (_, path) in listed {
        for rc in Session::run_configs_streaming(&path).ok()? {
            let pair = (rc.rules_workspace, rc.rules_surface, rc.rules_goal);
            if !out.contains(&pair) {
                out.push(pair);
            }
        }
        if !wanted.is_empty()
            && wanted
                .iter()
                .all(|w| out.iter().any(|p| pair_presents(p, w)))
        {
            return Some(out);
        }
    }
    // A listing that yielded no record carrying any key cannot answer
    // the question: a missing or empty `sessions/` lists as `Ok(empty)`,
    // and a record from before the fields carries `(None, None, None)` — "before
    // the field" and "matched with no key" are two facts the record keeps
    // apart, and a whole store of the first kind (every record on this
    // machine, the day the keys landed) read as evidence that every keyed
    // scope loads nowhere (found on review). Zero records is the least
    // evidence there is; it must not make the loudest claim.
    if out
        .iter()
        .all(|(w, k, g)| w.is_none() && k.is_none() && g.is_none())
    {
        return None;
    }
    Some(out)
}

/// Each goal an active rule's scope names, by its `kind:id`, with what the
/// store that owns it says (R34): `task:` against the board, `trigger:`
/// against the trigger store. What the roster's `LOADS NOWHERE` reads for
/// the goal key, since the presented-keys walk cannot see a goal that has
/// closed — the run that mined the rule always presented it.
pub(crate) type Goals = BTreeMap<String, GoalLiveness>;

/// The board and the trigger lookup R34's readout reads, shared by the
/// roster and `mecha learn`'s log. The board is read over MCP (`kg_task_list`
/// with closed rows), so only when `need_board` — some key in play names a
/// task — and under a deadline; a board that could not be read comes back
/// as the reason, which every `task:` goal then reports as unknown. The
/// trigger store is read in place and never created.
pub(crate) async fn goal_stores(
    global: &crate::GlobalOpts,
    need_board: bool,
) -> (
    std::result::Result<BoardStatuses, String>,
    impl Fn(&str) -> TriggerState,
) {
    let board = if need_board {
        match tokio::time::timeout(
            std::time::Duration::from_secs(BOARD_DEADLINE_SECS),
            super::tasks::read_board(global),
        )
        .await
        {
            Err(_) => Err(format!(
                "`kg_task_list` did not answer within {BOARD_DEADLINE_SECS} s"
            )),
            Ok(Err(e)) => Err(format!("{e:#}")),
            Ok(Ok(answer)) => BoardStatuses::of(&answer),
        }
    } else {
        Err("the board was not read on this path".to_string())
    };
    let root = mecha_core::trigger::TriggerStore::default_root().map_err(|e| format!("{e:#}"));
    let trigger = move |name: &str| match &root {
        Ok(root) => mecha_core::trigger::TriggerStore::state_at(root, name),
        Err(why) => TriggerState::Unreadable(format!("the trigger store: {why}")),
    };
    (board, trigger)
}

/// How long the terminal roster and the learn log wait on the board: the
/// connection is an MCP server start. The TUI and the web settings page
/// never wait on it — their budgets cannot fit one (the TUI blocks its
/// event loop on the shell-out; the web gives it ten seconds), so they pass
/// `--no-board` (found on review).
const BOARD_DEADLINE_SECS: u64 = 20;

/// [`Goals`] for the active rules of `rules`, reading the board only when
/// one of them names a task and `board` allows it — without it a task goal
/// is unknown, never open.
async fn goals_of(global: &crate::GlobalOpts, rules: &[Rule], board: bool) -> Goals {
    let named: Vec<GoalKey> = rules
        .iter()
        .filter(|r| r.active())
        .filter_map(|r| r.scope.as_ref().and_then(|s| s.goal.clone()))
        .filter(|g| g.named().is_some())
        .collect();
    if named.is_empty() {
        return Goals::new();
    }
    let read = board && named.iter().any(GoalKey::needs_board);
    let (board, trigger) = goal_stores(global, read).await;
    let mut out = Goals::new();
    for g in named {
        out.entry(g.to_string())
            .or_insert_with(|| g.liveness(&board, &trigger));
    }
    out
}

/// Every rule in the store, user and learned, in domain order.
fn all_rules(store: &LearningStore) -> Vec<Rule> {
    store
        .domains()
        .into_iter()
        .flat_map(|d| {
            store
                .user_rules(&d)
                .unwrap_or_default()
                .into_iter()
                .chain(store.learned_rules(&d).unwrap_or_default())
        })
        .collect()
}

/// What [`Goals`] says about a rule's goal, when its scope names one.
fn goal_of<'a>(r: &Rule, goals: &'a Goals) -> Option<&'a GoalLiveness> {
    r.scope
        .as_ref()
        .and_then(|s| s.goal.as_ref())
        .and_then(|g| goals.get(&g.to_string()))
}

/// Whether an active rule's scope names a workspace, surface or goal no run
/// record presented — `Some(false)` by construction for a scope naming
/// none of them, `None` when the store could not be read in full (unknown is
/// not nowhere), and never `Some(true)` for a retired rule. A goal the store
/// says has closed loads nowhere whatever the records presented (R34), and a
/// goal the store could not answer for turns a "loads" into unknown — an
/// unreadable board never reads as "not dark". One helper for the roster's
/// prose and its JSON, so the two cannot drift.
fn loads_nowhere(r: &Rule, keys: Option<&Presented>, goals: &Goals) -> Option<bool> {
    let scope = r.scope.as_ref()?.scope();
    // A parked surface provably matches no run, whatever the store holds —
    // and so does a corpus-mark surface on the stored scope, which
    // `scope()` strips but `matches` refuses, since no front-end declares
    // one; the roster must agree with the startup warning (found on
    // review).
    // A parked goal is the same: `Situation::matches` refuses it outright.
    if scope.surface_unread.is_some()
        || matches!(scope.goal, Some(GoalKey::Unread(_)))
        || r.scope
            .as_ref()
            .and_then(|s| s.surface)
            .is_some_and(|k| mecha_core::situation::Situation::MARK_KINDS.contains(&k))
    {
        return Some(r.active());
    }
    let goal = goal_of(r, goals);
    if let Some(GoalLiveness::Closed(_)) = goal {
        return Some(r.active());
    }
    let answer = if scope.workspace.is_none() && scope.surface.is_none() && scope.goal.is_none() {
        Some(false)
    } else {
        keys.map(|k| r.active() && !presented(&scope, k))
    };
    match goal {
        Some(GoalLiveness::Unknown(_)) if r.active() && answer == Some(false) => None,
        _ => answer,
    }
}

/// Whether some run record presents every workspace/surface/goal key `scope`
/// names — the corpus-shaped half of `unloadable_rules`, which names tools
/// only: a rule scoped to a workspace, surface or goal no `prepare` ever
/// matched against is dark with nothing warning, and the only honest test is
/// the record of what runs actually presented (found on review). A scope
/// that names none of the three keys is presented by construction.
fn presented(scope: &mecha_core::situation::Situation, presented: &Presented) -> bool {
    // A parked surface or goal is a key `Situation::matches` refuses outright.
    if scope.surface_unread.is_some() || matches!(scope.goal, Some(GoalKey::Unread(_))) {
        return false;
    }
    if scope.workspace.is_none() && scope.surface.is_none() && scope.goal.is_none() {
        return true;
    }
    presented.iter().any(|p| {
        pair_presents(
            p,
            &(scope.workspace.clone(), scope.surface, scope.goal.clone()),
        )
    })
}

/// Row 2e-5b and 2e-5c, per rule: its owner record over the runs that
/// carried it, and whether its region has gone quiet. Read once per roster;
/// `None` at the call site is a path that skipped the walk (`--no-board`),
/// which says "not read", never a standing.
struct Standing {
    tally: Tally,
    /// `None` when the session store could not be walked.
    recurrence: Option<Recurrence>,
    now: chrono::DateTime<chrono::Utc>,
}

impl Standing {
    fn read(rules: &[Rule], now: chrono::DateTime<chrono::Utc>) -> Standing {
        let wanted: std::collections::BTreeSet<String> = rules
            .iter()
            .filter(|r| r.active())
            .filter_map(|r| r.id.clone())
            .collect();
        // No active learned rule, nothing to report: no walk, and no
        // stand-in walk either — an empty one would read as evidence.
        let recurrence = if wanted.is_empty() {
            None
        } else {
            Session::default_dir()
                .ok()
                .and_then(|dir| Recurrence::scan(&dir, now).ok())
        };
        Standing {
            tally: owner_tally(&wanted),
            recurrence,
            now,
        }
    }

    fn quiet(&self, r: &Rule) -> Quiet {
        Quiet::of(r, self.recurrence.as_ref(), self.now)
    }
}

/// The owner's verdicts on the runs that carried each of `wanted`, over
/// the default session store and appraisal stores.
fn owner_tally(wanted: &std::collections::BTreeSet<String>) -> Tally {
    if wanted.is_empty() {
        return Tally::default();
    }
    match Session::default_dir() {
        Ok(dir) => Tally::scan(&dir, &mecha_core::appraisal::Stores::load(), wanted),
        Err(e) => Tally {
            store_unreadable: Some(format!("the session store could not be found ({e:#})")),
            ..Tally::default()
        },
    }
}

/// The roster's second line for an active learned rule: where it stands
/// with the owner, and whether its region is quiet.
fn standing_line(r: &Rule, standing: &Standing) -> String {
    let owner = match r.id.as_deref() {
        None => "no id, so no run record names it".to_string(),
        Some(id) => {
            let record = standing.tally.record(id);
            let tenure = standing.tally.tenure(id);
            let leash = if r.probation && tenure.is_tenured() {
                " — the retirement scan gives it the ordinary leash, not probation's"
            } else {
                ""
            };
            format!("{}{leash}", tenure.describe(&record))
        }
    };
    format!(
        "      owner: {owner} · region: {}",
        standing.quiet(r).describe()
    )
}

fn list(
    store: &LearningStore,
    as_json: bool,
    goals: &Goals,
    standing: Option<&Standing>,
) -> Result<()> {
    let tallies = rule_tallies(&store.validations()?);
    let everything = all_rules(store);
    let keys = presented_keys(&everything.iter().collect::<Vec<_>>());
    if as_json {
        let mut out = Vec::new();
        for domain in store.domains() {
            // User rules ride in the same prompt and are **not on trial** —
            // they are the owner's and are never tallied or retired. Listed
            // anyway, and flagged, because a surface that shows only the
            // learned half misdescribes what a run actually carries.
            for (r, mine) in store
                .user_rules(&domain)?
                .iter()
                .map(|r| (r, true))
                .chain(store.learned_rules(&domain)?.iter().map(|r| (r, false)))
            {
                let tally = r.id.as_deref().and_then(|id| tallies.get(id));
                // Row 2e-5b/2e-5c. `null` for a user rule (never on trial),
                // and on a path that skipped the walk: not read, never
                // "no verdicts".
                let owner = standing.filter(|_| !mine && r.active()).and_then(|st| {
                    let id = r.id.as_deref()?;
                    let record = st.tally.record(id);
                    Some(serde_json::json!({
                        "tenure": st.tally.tenure(id),
                        "accepted": record.accepted,
                        "rejected": record.rejected,
                        "unread": record.unread,
                    }))
                });
                let quiet = standing
                    .filter(|_| !mine && r.active())
                    .map(|st| st.quiet(r));
                out.push(serde_json::json!({
                    "id": r.id,
                    "domain": domain,
                    "title": r.text,
                    "user": mine,
                    "active": r.active(),
                    "retired": r.retired_at.is_some(),
                    "retired_reason": r.retired_reason,
                    // Applied without the gate being able to grade it. The
                    // web roster shows it for the same reason the terminal
                    // does: "measured clean" and "not measured" are different
                    // states and only one of them earned its place.
                    "probation": r.probation,
                    // Where it loads: `null` is a rule from before scoping
                    // (everywhere), a string is `Situation::describe`.
                    "scope": r.scope.as_ref().map(|s| s.describe()),
                    // A scope no run record presents — dark everywhere,
                    // whatever `scope` says. `null` when the store could
                    // not be read: unknown is not "nowhere".
                    // Decided per rule: a scope naming none of the keys is
                    // presented by construction and says `false` whatever
                    // the store holds; one naming a key says `null` only
                    // when the store could not be read in full (found on
                    // review — `null` had also meant "no rule needed the
                    // walk").
                    "loads_nowhere": loads_nowhere(r, keys.as_ref(), goals),
                    // R34: the goal the scope names has closed (why), or
                    // the store that owns it could not say (why). Both
                    // `null` for a scope with no goal, or an open one.
                    "goal_closed": match goal_of(r, goals) {
                        Some(GoalLiveness::Closed(why)) => Some(why.clone()),
                        _ => None,
                    },
                    "goal_unknown": match goal_of(r, goals) {
                        Some(GoalLiveness::Unknown(why)) => Some(why.clone()),
                        _ => None,
                    },
                    // Where the evidence was seen to hold, and whether a
                    // scan ever narrowed it — see `Rule::support`.
                    "support": r.support.iter().map(|s| s.describe()).collect::<Vec<_>>(),
                    "narrowed_at": r.narrowed_at,
                    "narrowed_reason": r.narrowed_reason,
                    "observations": tally.map(|t| t.observations),
                    // Beside observations, because they answer different
                    // questions and the gap between them is the roster's
                    // only way to explain a covered rule still on probation.
                    "graded": tally.map(|t| t.graded),
                    "attributed_regressions": tally.map(|t| t.attributed_regressions),
                    // Per exercised sub-region; rows with no region are
                    // under `unknown`. See `ValidationRecord::region`.
                    "regions": tally.map(|t| {
                        t.regions
                            .values()
                            .map(|(region, c)| {
                                serde_json::json!({
                                    "region": region.describe(),
                                    "graded": c.graded,
                                    "improved": c.improved,
                                    "regressed": c.regressed,
                                    "attributed_regressions": c.attributed_regressions,
                                })
                            })
                            .collect::<Vec<_>>()
                    }),
                    "unknown_region_graded": tally.map(|t| t.unknown_region.graded),
                    "created_at": r.created_at,
                    "owner": owner,
                    "quiet": quiet,
                }));
            }
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    // R34: a rule toward a closed goal is marked where it is listed; the
    // count leads, so a roster of many rules cannot hide one.
    let closed = everything
        .iter()
        .filter(|r| r.active() && matches!(goal_of(r, goals), Some(GoalLiveness::Closed(_))))
        .count();
    if closed > 0 {
        println!(
            "{closed} active rule(s) are scoped to a goal that has closed and load nowhere \
             (marked LOADS NOWHERE below); each widens only when the lesson is restated \
             toward another goal"
        );
    }
    let mut unknown: BTreeMap<&str, usize> = BTreeMap::new();
    for r in everything.iter().filter(|r| r.active()) {
        if let Some(GoalLiveness::Unknown(why)) = goal_of(r, goals) {
            *unknown.entry(why.as_str()).or_default() += 1;
        }
    }
    for (why, n) in unknown {
        println!("whether {n} active rule(s) are dark is unknown: {why}");
    }
    let mut any = false;
    for domain in store.domains() {
        let user = store.user_rules(&domain)?;
        let learned = store.learned_rules(&domain)?;
        if user.is_empty() && learned.is_empty() {
            continue;
        }
        any = true;
        println!("## {domain}");
        if !user.is_empty() {
            println!("  {} user rule(s) — immutable, never tallied", user.len());
        }
        for r in &learned {
            println!("  {}", describe(r, &tallies, keys.as_ref(), goals));
            if let Some(st) = standing.filter(|_| r.active()) {
                println!("{}", standing_line(r, st));
            }
        }
    }
    if !any {
        println!("no rules yet — `mecha learn` creates them");
    }
    if let Some(st) = standing {
        let quiet = everything
            .iter()
            .filter(|r| r.active() && st.quiet(r) == Quiet::Quiet)
            .count();
        if quiet > 0 {
            println!(
                "{quiet} active rule(s) are QUIET: no run in their region in the last {} days. \
                 Reported only; they keep loading and keep their place.",
                mecha_core::replay_priority::RECURRENCE_WINDOW_DAYS
            );
        }
        match &st.recurrence {
            None if everything.iter().any(|r| r.active() && r.id.is_some()) => {
                println!("quiet regions unknown: the session store could not be walked")
            }
            None => {}
            Some(rec) if rec.unreadable > rec.unreadable_in_window => println!(
                "quiet regions: {} transcript(s) whose header could not be read have no date, \
                 so they are not counted in the window",
                rec.unreadable - rec.unreadable_in_window
            ),
            Some(_) => {}
        }
        for c in &st.tally.caveats {
            println!("owner tenure: {c}");
        }
        if let Some(why) = &st.tally.store_unreadable {
            println!("owner tenure unknown for every rule: {why}");
        }
    }
    Ok(())
}

/// Put back the probation mark the owner's tenure lifted for this pass, on
/// a set about to be written or staged: the release is the scan's, never
/// the file's (row 2e-5b).
fn restore_owner_marks(rules: &mut [Rule], released: &std::collections::BTreeSet<String>) {
    for r in rules.iter_mut() {
        if r.id.as_deref().is_some_and(|id| released.contains(id)) {
            r.probation = true;
        }
    }
}

/// Whether `p` already proposes the *same* change to rule `id` that
/// `verdict` would make: retirement, or a narrowing to the same scope,
/// relative to the proposal's own `rules_before`. A proposal that merely
/// carries the rule forward — every learn proposal does, for the whole
/// domain — proposes nothing about it; and one that *widened* it is a
/// scope change in the other direction, which a scope-changed test read as
/// a twin and would have superseded under `--apply` (found on review).
fn proposal_matches_verdict(p: &Proposal, id: &str, verdict: &Verdict) -> bool {
    let after = p.rules.iter().find(|r| r.id.as_deref() == Some(id));
    let before = p.rules_before.iter().find(|r| r.id.as_deref() == Some(id));
    // Absent before and present after is a new rule, not a change to this
    // one; absent after is the whole-rewrite drop, which is not a
    // retirement either.
    let (Some(b), Some(a)) = (before, after) else {
        return false;
    };
    match verdict {
        Verdict::Retire { .. } => a.retired_at.is_some() && b.retired_at.is_none(),
        Verdict::Narrow { scope, .. } => {
            a.retired_at.is_none() && a.scope.as_ref() == Some(scope) && b.scope != a.scope
        }
        Verdict::Stands => false,
    }
}

fn describe(
    r: &Rule,
    tallies: &BTreeMap<String, RuleTally>,
    keys: Option<&Presented>,
    goals: &Goals,
) -> String {
    let id =
        r.id.as_deref()
            .unwrap_or("(no id — predates identity; next learn pass mints one)");
    let state = if r.retired_at.is_some() {
        format!(
            "RETIRED {}{}",
            r.retired_at.as_deref().unwrap_or_default(),
            r.retired_reason
                .as_deref()
                .map(|w| format!(" — {w}"))
                .unwrap_or_default()
        )
    } else if !r.enabled {
        "disabled".into()
    } else if r.probation {
        // Visible, because a rule that went live ungraded is a different
        // thing to read than one that was measured clean, and the roster is
        // where the owner would notice the difference.
        format!(
            "active (probation — applied ungraded, retires at {})",
            mecha_core::learning::PROBATION_RETIRE_AT
        )
    } else {
        "active".into()
    };
    // Where the rule loads. Unscoped and standing both load everywhere, and
    // are printed apart because they are different facts about the evidence
    // — see `Rule::scope`.
    let scope = match &r.scope {
        None => "unscoped (predates scoping; loads everywhere)".to_string(),
        // `loads_nowhere` before `is_standing`: a scope naming only a corpus
        // mark is standing by `scope()`'s definition and refused by
        // `matches`, and the prose must agree with the JSON (found on review).
        Some(s) if loads_nowhere(r, keys, goals) == Some(true) => match goal_of(r, goals) {
            Some(GoalLiveness::Closed(why)) => format!(
                "scoped to {} — LOADS NOWHERE: {why}; it widens only when the lesson is \
                 restated toward another goal (R34)",
                s.describe()
            ),
            _ => format!(
                "scoped to {} — LOADS NOWHERE: no run record presents that workspace/surface/goal",
                s.describe()
            ),
        },
        Some(s) if s.is_standing() => "standing (loads everywhere)".to_string(),
        Some(s) => match goal_of(r, goals) {
            Some(GoalLiveness::Unknown(why)) if r.active() => format!(
                "loads with {} — whether its goal is still open is unknown: {why}",
                s.describe()
            ),
            _ => format!("loads with {}", s.describe()),
        },
    };
    // Where it was seen to hold, when that is more than where it loads —
    // a widened rule names each sub-region it widened over; a narrowed one
    // says what it shed.
    // Standing support is the old scope of a widened pre-field rule and
    // says nothing a `standing` scope does not already say.
    let seen: Vec<String> = r
        .support
        .iter()
        .filter(|s| !s.is_standing())
        .map(|s| s.describe())
        .collect();
    let support = if seen.len() > 1
        || r.support
            .iter()
            .find(|s| !s.is_standing())
            .is_some_and(|s| Some(s) != r.scope.as_ref())
    {
        format!(" · seen in {}", seen.join("; "))
    } else {
        String::new()
    };
    let narrowed = match (&r.narrowed_at, &r.narrowed_reason) {
        (Some(at), Some(why)) => format!(" · narrowed {at} — {why}"),
        (Some(at), None) => format!(" · narrowed {at}"),
        _ => String::new(),
    };
    // Three states, no two rendering alike: graded, ran-but-graded-nothing,
    // and never probed. Collapsing the middle into the first printed
    // "0 improved, 0 regressed" for inconclusive-only coverage — a clean
    // bill of health from rows that graded nothing, and the reason a
    // covered rule can still be on probation.
    let measured = match r.id.as_deref().and_then(|id| tallies.get(id)) {
        Some(t) if t.graded > 0 => format!(
            "{} probe(s), {} graded: {} improved, {} regressed, {} attributed to this rule; last {}{}",
            t.observations,
            t.graded,
            t.improved,
            t.regressed,
            t.attributed_regressions,
            t.last_validated.as_deref().unwrap_or("never"),
            regions_line(t)
        ),
        Some(t) if t.observations > 0 => format!(
            "{} probe(s) ran, none graded — inconclusive coverage measures nothing; last {}",
            t.observations,
            t.last_validated.as_deref().unwrap_or("never")
        ),
        _ => "never validated".into(),
    };
    format!(
        "[{state}] {}\n      id {id} · created {} · {scope}{support}{narrowed} · {measured}",
        r.text,
        r.created_at.as_deref().unwrap_or("unknown"),
    )
}

/// The graded rows split by the sub-region they exercised, after the
/// totals: `shell 4 graded (1 attributed) · fs_read 2 graded`, with rows
/// that named no region counted as such — unknown is not a region and is
/// never folded into one.
fn regions_line(t: &RuleTally) -> String {
    let mut parts: Vec<String> = t
        .regions
        .values()
        .filter(|(_, c)| c.graded > 0)
        .map(|(region, c)| {
            format!(
                "{} {} graded{}",
                region.describe(),
                c.graded,
                match c.attributed_regressions {
                    0 => String::new(),
                    n => format!(" ({n} attributed)"),
                }
            )
        })
        .collect();
    if t.unknown_region.graded > 0 {
        parts.push(format!(
            "no region recorded {} graded{}",
            t.unknown_region.graded,
            match t.unknown_region.attributed_regressions {
                0 => String::new(),
                n => format!(" ({n} attributed)"),
            }
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("\n      by region: {}", parts.join(" · "))
    }
}

/// Find one learned rule by id or unique prefix, returning its domain.
/// Ambiguity is an error rather than a guess, same as proposal lookup.
fn find_rule(store: &LearningStore, id: &str) -> Result<(String, Vec<Rule>, usize)> {
    // `rid.starts_with("")` is true for every rule that has an id, so an
    // empty needle — a TUI row whose `Rule::id` was `None`, serialised to
    // `null` and read back as `""` — would match every learned rule in
    // every domain instead of none. `mecha rules retire ""` is reachable
    // from the command line too.
    anyhow::ensure!(!id.is_empty(), "no rule id given");
    let mut hits: Vec<(String, Vec<Rule>, usize)> = Vec::new();
    for domain in store.domains() {
        let rules = store.learned_rules(&domain)?;
        for (i, r) in rules.iter().enumerate() {
            if r.id.as_deref().is_some_and(|rid| rid.starts_with(id)) {
                hits.push((domain.clone(), rules.clone(), i));
            }
        }
    }
    match hits.len() {
        0 => bail!("no learned rule matching `{id}` — `mecha rules` lists ids"),
        1 => Ok(hits.remove(0)),
        n => bail!("`{id}` matches {n} rules; give more of the id"),
    }
}

fn retire(store: &LearningStore, id: &str, reason: Option<String>) -> Result<()> {
    let _lock = store.lock()?;
    let (domain, mut rules, i) = find_rule(store, id)?;
    if rules[i].retired_at.is_some() {
        bail!(
            "rule {} is already retired",
            rules[i].id.as_deref().unwrap_or(id)
        );
    }
    rules[i].enabled = false;
    rules[i].retired_at = Some(chrono::Utc::now().to_rfc3339());
    rules[i].retired_reason = Some(reason.clone().unwrap_or_else(|| "retired by hand".into()));
    store.write_learned_rules(&domain, &rules)?;
    // The owner's verdict on the rule's tenure (R16f), recorded against the
    // rule: `retired_at` alone cannot say who retired it — the retirement
    // scan writes the same field — and a restore clears it.
    record_owner_verdict(store, &rules[i], mecha_core::curation::Act::Retired, reason)?;
    store.commit(&format!(
        "retire[{domain}]: {}",
        rules[i].id.as_deref().unwrap_or(id)
    ));
    println!("retired from `{domain}`: {}", rules[i].text);
    Ok(())
}

fn restore(store: &LearningStore, id: &str) -> Result<()> {
    let _lock = store.lock()?;
    let (domain, mut rules, i) = find_rule(store, id)?;
    if rules[i].retired_at.is_none() {
        bail!(
            "rule {} is not retired",
            rules[i].id.as_deref().unwrap_or(id)
        );
    }
    rules[i].enabled = true;
    rules[i].retired_at = None;
    rules[i].retired_reason = None;
    store.write_learned_rules(&domain, &rules)?;
    record_owner_verdict(store, &rules[i], mecha_core::curation::Act::Restored, None)?;
    store.commit(&format!(
        "restore[{domain}]: {}",
        rules[i].id.as_deref().unwrap_or(id)
    ));
    println!("restored to `{domain}`: {}", rules[i].text);
    Ok(())
}

/// Append the owner's verdict on `rule` to the learning store's curation
/// ledger (`mecha_core::curation`), under the lock the caller holds. After
/// the rules file is written, so a verdict is never recorded for a change
/// that did not land; a ledger that cannot be written fails the command
/// rather than leaving the tenure record short without a word.
fn record_owner_verdict(
    store: &LearningStore,
    rule: &Rule,
    act: mecha_core::curation::Act,
    reason: Option<String>,
) -> Result<()> {
    let Some(id) = rule.id.clone() else {
        // `find_rule` only matches rules with an id; nothing to record
        // against otherwise.
        return Ok(());
    };
    mecha_core::curation::append(
        store.root(),
        &mecha_core::curation::Verdict::now(mecha_core::curation::Target::Rule(id), act, reason),
    )
    .context("the rule changed, but the owner's verdict could not be recorded against it")
}

fn show(store: &LearningStore, id: &str, goals: &Goals, standing: Option<&Standing>) -> Result<()> {
    let tallies = rule_tallies(&store.validations()?);
    let (domain, rules, i) = find_rule(store, id)?;
    let keys = presented_keys(&[&rules[i]]);
    println!(
        "## {domain}\n{}",
        describe(&rules[i], &tallies, keys.as_ref(), goals)
    );
    if let Some(st) = standing.filter(|_| rules[i].active()) {
        println!("{}", standing_line(&rules[i], st));
    }
    Ok(())
}

fn propose(
    store: &LearningStore,
    min_attributed: u32,
    apply: bool,
    owner: Option<&Tally>,
) -> Result<()> {
    let _lock = store.lock()?;
    let records = store.validations()?;
    let tallies = rule_tallies(&records);
    let proposals = store.proposals()?;
    let mut staged = 0u32;

    for domain in store.domains() {
        let mut before = store.learned_rules(&domain)?;
        // Probation says "born ungraded", which stops being true once the
        // ledger grades the rule beyond its convictions. Released before the
        // threshold is chosen, or a rule with a real clean record would still
        // answer to the short leash — the stale-stamp failure, one store
        // over. The release deliberately does not key on bare coverage: an
        // attributed regression always arrives inside an observation, so
        // that release stripped the leash on the very rows that convict and
        // made PROBATION_RETIRE_AT unreachable from this scan.
        mecha_core::learning::release_probation_when_measured_clean(&mut before, &tallies);
        // Beside it, never instead (row 2e-5b, R41): a rule the owner's
        // verdicts have tenured answers to the ordinary leash too. The
        // retirement below is still decided by measured regressions alone.
        //
        // **Per pass, and the file keeps the mark.** The bound is not
        // monotone — rejects arriving later can take a rule back under the
        // floor — so a release written to disk would outlive the record
        // that argued for it. The ids released here get their mark back on
        // every set this scan writes or stages (`restore_owner_marks`);
        // the ledger's release, whose condition only accumulates, is left
        // as it was (found on review of #338).
        let mut owner_released: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        if let Some(owner) = owner {
            let on_probation: std::collections::BTreeSet<String> = before
                .iter()
                .filter(|r| r.probation)
                .filter_map(|r| r.id.clone())
                .collect();
            mecha_core::tenure::release_probation_when_owner_tenures(&mut before, owner);
            owner_released = before
                .iter()
                .filter(|r| !r.probation)
                .filter_map(|r| r.id.clone())
                .filter(|id| on_probation.contains(id))
                .collect();
            if !owner_released.is_empty() {
                println!(
                    "{domain}: {} probationary rule(s) tenured by the owner's verdicts \
                     answer to the ordinary leash this pass",
                    owner_released.len()
                );
            }
        }
        let before = before;
        // Per rule: stands, narrows, or retires. Narrowing is retirement's
        // gentler sibling (`judge_convicted`): a rule convicted in one of
        // the sub-regions it was seen in and clean in the others sheds the
        // failing one and keeps loading where it held.
        let verdicts: Vec<(&Rule, Verdict)> = before
            .iter()
            .filter(|r| r.active())
            .filter_map(|r| {
                // Per-rule, not per-pass: a probationary rule went live
                // ungraded and answers to a shorter leash.
                let threshold = retire_threshold_for(r, min_attributed);
                // The ledger since the rule's last narrowing (`tally_for`):
                // the rows before it were the evidence it answered.
                let tally = tally_for(r, &records)?;
                match judge_convicted(r, &tally, threshold) {
                    Verdict::Stands => None,
                    v => Some((r, v)),
                }
            })
            .collect();
        if verdicts.is_empty() {
            continue;
        }
        let convicted: Vec<&Rule> = verdicts.iter().map(|(r, _)| *r).collect();
        // A pending proposal already retiring or narrowing these exact
        // rules is not re-staged — the nightly must not spam the queue
        // while a human hasn't looked yet. Collected rather than tested,
        // because the apply path below owes each of these a resolution.
        //
        // A twin is a proposal that *changes* each convicted rule — retires
        // it, or moves its scope — relative to its own `rules_before`.
        // `narrowed_at` alone is not the test: it is a durable flag that
        // rides on an active rule forever, so a later `learn --propose`
        // proposal, which writes the whole domain's set, would carry a
        // once-narrowed rule unchanged and read as a twin — and under
        // `--apply` be marked superseded, discarding a consolidation that
        // was waiting on the owner (found on review).
        let convicted_ids: Vec<&str> = convicted.iter().filter_map(|r| r.id.as_deref()).collect();
        let pending_twins: Vec<mecha_core::learning::Proposal> = proposals
            .iter()
            .filter(|p| {
                p.status == "pending"
                    && p.domain == domain
                    && verdicts.iter().all(|(r, v)| {
                        r.id.as_deref()
                            .is_some_and(|id| proposal_matches_verdict(p, id, v))
                    })
            })
            .cloned()
            .collect();
        if !pending_twins.is_empty() && !apply {
            println!("{domain}: retirement already pending — review with `mecha proposals`");
            continue;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut evidence_lines = Vec::new();
        let mut narrowed_count = 0u32;
        let rules: Vec<Rule> = before
            .iter()
            .map(|r| {
                let Some((_, verdict)) = verdicts
                    .iter()
                    .find(|(c, _)| c.id.is_some() && c.id == r.id)
                else {
                    return r.clone();
                };
                let t = tally_for(r, &records).expect("convicted, so charged");
                let against = t.attributed_regressions;
                evidence_lines.push(format!(
                    "{}: {} attributed regression(s) {} across {} \
                     probe(s) ({} improved, {} regressed at block level); last validated {}\n  rule: {}",
                    r.id.as_deref().unwrap(),
                    against,
                    match &r.narrowed_at {
                        Some(at) => format!("since it was narrowed at {at}"),
                        None => "in the validation ledger".to_string(),
                    },
                    t.observations,
                    t.improved,
                    t.regressed,
                    t.last_validated.as_deref().unwrap_or("never"),
                    r.text,
                ));
                let leash = match r.probation {
                    true => format!(
                        " (probation: retires at {})",
                        retire_threshold_for(r, min_attributed)
                    ),
                    false => String::new(),
                };
                match verdict {
                    Verdict::Narrow {
                        scope,
                        support,
                        shed,
                    } => {
                        narrowed_count += 1;
                        let why = format!(
                            "{against} attributed regression(s) in {}{leash}; kept where it \
                             held ({})",
                            shed.iter().map(|s| s.describe()).collect::<Vec<_>>().join("; "),
                            support.iter().map(|s| s.describe()).collect::<Vec<_>>().join("; "),
                        );
                        evidence_lines.push(format!(
                            "  narrowed: loads with {} — {why}",
                            scope.describe()
                        ));
                        let mut narrowed = r.clone();
                        narrowed.scope = Some(scope.clone());
                        narrowed.support = support.clone();
                        narrowed.narrowed_at = Some(now.clone());
                        narrowed.narrowed_reason = Some(why);
                        narrowed
                    }
                    Verdict::Retire { why } => {
                        evidence_lines.push(format!("  retired: {why}"));
                        let mut retired = r.clone();
                        retired.enabled = false;
                        retired.retired_at = Some(now.clone());
                        // Name the shorter leash when it is what convicted,
                        // or the record reads as though the ordinary
                        // threshold was met.
                        retired.retired_reason =
                            Some(format!("{why}{leash}"));
                        retired
                    }
                    Verdict::Stands => unreachable!("filtered above"),
                }
            })
            .collect();
        let mut rules = rules;
        restore_owner_marks(&mut rules, &owner_released);
        let retired_count = convicted.len() as u32 - narrowed_count;
        evidence_lines.push(format!(
            "deterministic ledger scan over {} record(s); threshold {min_attributed} \
             attributed regression(s); no model involved",
            records
                .iter()
                .filter(|rec: &&ValidationRecord| rec.domain == domain)
                .count(),
        ));

        // ── the direct path: write the retirement, no queue, no human ──
        //
        // A retired rule is disabled in place and keeps `retired_at` /
        // `retired_reason`, so this removes it from every future prompt
        // without removing it from the record — the learner is still told it
        // was tried and measured harmful, which is what stops it being
        // re-derived. That is the mechanism `git revert` was standing in for,
        // and unlike a revert it is per-rule and leaves the rest of the store
        // alone.
        if apply {
            store.write_learned_rules(&domain, &rules)?;
            store.append_run(&LeapRun {
                id: Session::new_id(),
                domain: domain.clone(),
                reflexions_processed: 0,
                // **Whole file, not the active subset** — the count every
                // other `LeapRun` writer uses (`learn` writes
                // `learned_before.len()` / `rules.len()`; `accept` the same).
                // A retirement never removes a row, so these are equal and
                // the pass shows as a flat step; counting `active()` here
                // instead put two different measures on one series in the
                // "Rule set over time" chart, where a retirement would read
                // as a drop and a consolidation as a total.
                rules_before: before.len() as u32,
                rules_after: rules.len() as u32,
                created_at: now.clone(),
            })?;
            store.commit(&format!(
                "retire[{domain}]: {retired_count} retired, {narrowed_count} narrowed at \
                 {min_attributed}+ attributed regression(s)"
            ));
            println!(
                "{domain}: retired {retired_count} rule(s), narrowed {narrowed_count} — {}",
                convicted_ids.join(", ")
            );
            // Two lines per convicted rule: the tally and the verdict.
            for line in evidence_lines.iter().take(convicted.len() * 2) {
                println!("  {}", line.replace('\n', "\n  "));
            }
            // The pending twin resolves, not lingers: an applied retirement
            // leaves nothing for a human to decide, and a proposal still
            // `pending` after its content landed reads as awaiting review —
            // to `mecha proposals` and to doctor — forever. Superseded, not
            // accepted: nobody ruled on the paper, the direct path overtook
            // it. A twin is one whose only change to these rules is the
            // retirement or narrowing applied here (`proposal_matches_verdict`),
            // and such a proposal holds no reflections, so there is nothing
            // to release.
            for mut p in pending_twins {
                p.status = "superseded".into();
                p.resolved_at = Some(now.clone());
                p.reason =
                    Some("superseded: the same retirement was applied directly (--apply)".into());
                store.write_proposal(&p)?;
            }
            staged += 1;
            continue;
        }

        let proposal = Proposal {
            id: Session::new_id(),
            domain: domain.clone(),
            status: "pending".into(),
            // No reflections are consumed: retirement argues from the
            // ledger, and the rules' own sources stay marked as they were.
            reflexion_ids: Vec::new(),
            rules_before: {
                let mut was = before.clone();
                restore_owner_marks(&mut was, &owner_released);
                was
            },
            rules,
            evidence: evidence_lines.join("\n"),
            created_at: now,
            resolved_at: None,
            reason: None,
            // A retirement argues about the whole domain, not one region.
            scope: None,
        };
        store.write_proposal(&proposal)?;
        store.commit(&format!(
            "propose-retirement[{domain}]: {retired_count} to retire, {narrowed_count} to narrow \
             — {}",
            proposal.id
        ));
        println!(
            "{domain}: proposal {} retires {retired_count} and narrows {narrowed_count} rule(s) \
             — review with `mecha proposals show {}`",
            proposal.id, proposal.id
        );
        staged += 1;
    }
    if staged == 0 {
        println!(
            "no rule has {min_attributed}+ attributed regressions — nothing to retire \
             (`mecha rules` shows the tallies)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::learning::rules_hash;

    fn temp_store() -> LearningStore {
        // A process-unique counter, not a timestamp. `as_nanos()` is only as
        // fine-grained as the platform's clock: on macOS two of these called
        // from parallel test threads can land on the same value, and then two
        // tests share one directory — the first to finish `remove_dir_all`s
        // the other's store out from under it, which surfaces as a bare
        // `No such file or directory` in whichever test lost. Found on the
        // macOS CI arm, where it is a race rather than a certainty; it passed
        // twice before it failed.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join("mecha-rules-test")
            .join(format!("{}-{seq}", std::process::id()));
        // **Cleared, not just uniquely named.** The counter guarantees no two
        // stores in *this* process collide; it says nothing about a directory
        // a previous run left behind, and `{pid}-{seq}` is deterministic, so a
        // later run drawing the same pid reopens it. These stores append, so
        // the leftover records would be counted alongside the new ones — a
        // confusing count mismatch rather than a clean failure. Removing first
        // makes the fixture fresh regardless of what any earlier run did.
        let _ = std::fs::remove_dir_all(&dir);
        LearningStore::open(dir).unwrap()
    }

    fn rule(text: &str, id: &str) -> Rule {
        Rule {
            text: text.into(),
            id: Some(id.into()),
            ..Default::default()
        }
    }

    fn regression(rule_id: &str, at: &str) -> ValidationRecord {
        ValidationRecord {
            reflexion_id: "refl".into(),
            trigger: "steer".into(),
            domain: "behavior".into(),
            rules_hash: rules_hash("block"),
            rule_ids: vec![rule_id.into()],
            outcome: "regressed".into(),
            attributed_rule_id: Some(rule_id.into()),
            model: "qwen".into(),
            created_at: at.into(),
            region: None,
        }
    }

    #[test]
    fn retirement_is_proposed_at_the_threshold_and_through_the_gate() {
        let store = temp_store();
        store
            .write_learned_rules(
                "behavior",
                &[rule("Bad rule.", "r-bad"), rule("Fine rule.", "r-ok")],
            )
            .unwrap();
        for i in 0..3 {
            store
                .append_validation(&regression(
                    "r-bad",
                    &format!("2026-08-0{}T00:00:00Z", i + 1),
                ))
                .unwrap();
        }

        // Below threshold: nothing staged.
        propose(&store, 4, false, None).unwrap();
        assert!(store.proposals().unwrap().is_empty());

        // At threshold: one pending proposal that retires r-bad, keeps r-ok,
        // and consumes no reflections. The live rules must be untouched —
        // only acceptance deploys.
        propose(&store, 3, false, None).unwrap();
        let all = store.proposals().unwrap();
        assert_eq!(all.len(), 1);
        let p = &all[0];
        assert_eq!(p.status, "pending");
        assert!(p.reflexion_ids.is_empty());
        let bad = p
            .rules
            .iter()
            .find(|r| r.id.as_deref() == Some("r-bad"))
            .unwrap();
        assert!(bad.retired_at.is_some() && !bad.enabled);
        assert!(bad
            .retired_reason
            .as_deref()
            .unwrap()
            .contains("3 attributed"));
        assert!(p
            .rules
            .iter()
            .find(|r| r.id.as_deref() == Some("r-ok"))
            .unwrap()
            .active());
        assert!(p.evidence.contains("no model involved"));
        let live = store.learned_rules("behavior").unwrap();
        assert!(
            live.iter().all(|r| r.active()),
            "staging must not touch the live rules"
        );

        // Re-running while the proposal is pending must not stage a twin.
        propose(&store, 3, false, None).unwrap();
        assert_eq!(store.proposals().unwrap().len(), 1);

        std::fs::remove_dir_all(store.root()).ok();
    }

    /// A pending retirement proposal overtaken by `--apply` resolves as
    /// superseded rather than lingering. Fails on the old behaviour: the
    /// direct path retired the rule and left the paper `pending`, so
    /// `mecha proposals` and doctor read an already-applied retirement as
    /// awaiting review forever.
    #[test]
    fn apply_resolves_the_pending_twin_it_overtakes() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("Bad rule.", "r-bad")])
            .unwrap();
        for i in 0..3 {
            store
                .append_validation(&regression(
                    "r-bad",
                    &format!("2026-08-0{}T00:00:00Z", i + 1),
                ))
                .unwrap();
        }

        // Staged first, as the nightly would have before --apply existed.
        propose(&store, 3, false, None).unwrap();
        assert_eq!(store.proposals().unwrap()[0].status, "pending");

        // The direct path retires the rule and resolves the paper.
        propose(&store, 3, true, None).unwrap();
        let all = store.proposals().unwrap();
        assert_eq!(all.len(), 1, "no twin staged");
        assert_eq!(all[0].status, "superseded");
        assert!(all[0].resolved_at.is_some());
        assert!(all[0].reason.as_deref().unwrap().contains("--apply"));
        assert!(
            !store.learned_rules("behavior").unwrap()[0].active(),
            "the retirement itself landed"
        );

        std::fs::remove_dir_all(store.root()).ok();
    }

    /// **Retirement removes a rule from the live set with no queue and no
    /// human.** Until this existed, a rule measured harmful reached
    /// `retired_at` only inside a *proposal*, and the only thing that ever
    /// took one out of a prompt was `git revert` over the whole store — a
    /// whole-store undo standing in for a per-rule mechanism.
    ///
    /// Fails on the old behaviour: `--apply` did not exist, so the live rules
    /// were untouched by any scan and `r-bad` stayed active forever.
    #[test]
    fn retirement_applied_directly_disables_the_rule_and_leaves_the_rest_alone() {
        let store = temp_store();
        store
            .write_learned_rules(
                "behavior",
                &[rule("Bad rule.", "r-bad"), rule("Fine rule.", "r-ok")],
            )
            .unwrap();
        for i in 0..3 {
            store
                .append_validation(&regression(
                    "r-bad",
                    &format!("2026-08-0{}T00:00:00Z", i + 1),
                ))
                .unwrap();
        }

        // Below threshold, --apply must be as inert as staging is.
        propose(&store, 4, true, None).unwrap();
        assert!(
            store
                .learned_rules("behavior")
                .unwrap()
                .iter()
                .all(|r| r.active()),
            "an unconvicted rule must survive an --apply scan"
        );

        propose(&store, 3, true, None).unwrap();

        // The live file moved, and nothing was queued for anyone to accept.
        assert!(
            store.proposals().unwrap().is_empty(),
            "--apply must not also stage a proposal"
        );
        let live = store.learned_rules("behavior").unwrap();
        let bad = live
            .iter()
            .find(|r| r.id.as_deref() == Some("r-bad"))
            .unwrap();
        assert!(!bad.active(), "the convicted rule must leave the prompt");
        assert!(bad.retired_at.is_some());
        assert!(bad
            .retired_reason
            .as_deref()
            .unwrap()
            .contains("3 attributed"));

        // Retired, not deleted: it stays as evidence so the learner is told it
        // was tried and measured harmful, which is what stops re-derivation.
        assert_eq!(live.len(), 2, "a retired rule stays in the file");
        assert!(
            live.iter()
                .find(|r| r.id.as_deref() == Some("r-ok"))
                .unwrap()
                .active(),
            "retirement must be per-rule, not a whole-store revert"
        );

        // And it is recorded as a pass, so `git log` in the store reads as the
        // system's learning history rather than an unexplained file change.
        // Counted over the whole file, like every other `LeapRun` writer — a
        // retirement disables a rule without removing its row, so both are 2
        // and the pass reads as a flat step. Counting the *active* subset here
        // would put two different measures on one chart series.
        let runs = std::fs::read_to_string(store.root().join("runs.jsonl")).unwrap();
        assert!(
            runs.contains("\"rules_before\":2") && runs.contains("\"rules_after\":2"),
            "a retirement pass records the file's rule count, not the active one: {runs}"
        );

        std::fs::remove_dir_all(store.root()).ok();
    }
    /// **§17.4 end to end: the nightly scan narrows a widened rule to where
    /// it held and retires one it cannot narrow, in one pass, and the next
    /// pass leaves the narrowed rule alone.** Fails on the pre-narrowing
    /// scan, which retired both.
    #[test]
    fn the_scan_narrows_where_it_can_and_retires_where_it_cannot() {
        use mecha_core::situation::Situation;
        let sit = |tools: &[&str]| {
            Situation::of_run(
                &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                None,
            )
        };
        let store = temp_store();
        let widened = Rule {
            scope: Some(Situation::default()),
            support: vec![sit(&["shell"]), sit(&["http_fetch"])],
            ..rule("Widened rule.", "r-wide")
        };
        store
            .write_learned_rules("behavior", &[widened, rule("Bare rule.", "r-bare")])
            .unwrap();
        let placed = |rule_id: &str, tools: &[&str], outcome: &str, attributed: bool, at: &str| {
            ValidationRecord {
                outcome: outcome.into(),
                attributed_rule_id: attributed.then(|| rule_id.into()),
                region: Some(sit(tools)),
                ..regression(rule_id, at)
            }
        };
        for i in 0..3 {
            let at = format!("2026-09-0{}T00:00:00Z", i + 1);
            store
                .append_validation(&placed(
                    "r-wide",
                    &["shell", "fs_read"],
                    "regressed",
                    true,
                    &at,
                ))
                .unwrap();
            store.append_validation(&regression("r-bare", &at)).unwrap();
        }
        store
            .append_validation(&placed(
                "r-wide",
                &["http_fetch"],
                "unchanged_pass",
                false,
                "2026-09-04T00:00:00Z",
            ))
            .unwrap();

        propose(&store, 3, true, None).unwrap();

        let live = store.learned_rules("behavior").unwrap();
        let wide = live
            .iter()
            .find(|r| r.id.as_deref() == Some("r-wide"))
            .unwrap();
        assert!(wide.active(), "narrowed, not retired");
        assert_eq!(wide.scope, Some(sit(&["http_fetch"])));
        assert_eq!(wide.support, vec![sit(&["http_fetch"])]);
        assert!(wide.narrowed_at.is_some());
        let why = wide.narrowed_reason.as_deref().unwrap();
        assert!(why.contains("shell") && why.contains("http_fetch"), "{why}");
        let bare = live
            .iter()
            .find(|r| r.id.as_deref() == Some("r-bare"))
            .unwrap();
        assert!(!bare.active());
        assert!(bare
            .retired_reason
            .as_deref()
            .unwrap()
            .contains("no recorded support"));
        let runs = std::fs::read_to_string(store.root().join("runs.jsonl")).unwrap();
        assert!(runs.contains("\"rules_before\":2"));

        // The convictions that narrowed it lie outside where it now loads:
        // a second scan finds nothing against it.
        propose(&store, 3, true, None).unwrap();
        let again = store.learned_rules("behavior").unwrap();
        let wide = again
            .iter()
            .find(|r| r.id.as_deref() == Some("r-wide"))
            .unwrap();
        assert!(wide.active());
        assert_eq!(wide.scope, Some(sit(&["http_fetch"])));

        // And the roster says so, in prose.
        let tallies = rule_tallies(&store.validations().unwrap());
        let line = describe(wide, &tallies, None, &Goals::new());
        assert!(line.contains("loads with http_fetch"), "{line}");
        assert!(line.contains("narrowed "), "{line}");
        // A widened pre-field rule — support `[standing, shell]`, scope
        // unset — still says where it was seen (found on review).
        let pre_field = Rule {
            support: vec![Situation::default(), sit(&["shell"])],
            ..rule("Old and widened.", "r-pre")
        };
        let pre_line = describe(&pre_field, &tallies, None, &Goals::new());
        assert!(pre_line.contains("seen in shell"), "{pre_line}");
        assert!(!pre_line.contains("seen in everywhere"), "{pre_line}");
        assert!(line.contains("by region:"), "{line}");
        assert!(
            line.contains("fs_read, shell 3 graded (3 attributed)"),
            "{line}"
        );
        std::fs::remove_dir_all(store.root()).ok();
    }

    /// R34: a rule toward a goal that has closed loads nowhere although the
    /// run that mined it presented its goal — the case the presented-keys
    /// walk is blind to. An open goal loads; a retired rule is never
    /// flagged; and a goal whose store could not answer turns "loads" into
    /// unknown, never into "not dark". Fails on the old roster, which read
    /// the closed-task rule as loading.
    #[test]
    fn a_rule_toward_a_closed_goal_loads_nowhere_and_an_unread_board_is_unknown() {
        use mecha_core::situation::Situation;
        let w = PathBuf::from("/w");
        let task = |g: &str| Some(GoalKey::Named(g.parse().unwrap()));
        let keys: Presented = vec![
            (
                Some(w.clone()),
                Some(SessionKind::Task),
                task("task:t-done"),
            ),
            (
                Some(w.clone()),
                Some(SessionKind::Task),
                task("task:t-open"),
            ),
        ];
        let toward = |id: &str, g: &str| Rule {
            text: format!("Rule {id}."),
            id: Some(id.into()),
            scope: Some(
                Situation::of_run(&["shell".into()], Some(&w))
                    .on(Some(SessionKind::Task))
                    .toward(task(g))
                    .scope(),
            ),
            ..Default::default()
        };
        let done = toward("r-done", "task:t-done");
        let open = toward("r-open", "task:t-open");
        // The old answer, with nothing read about the goal: it loads.
        assert_eq!(
            loads_nowhere(&done, Some(&keys), &Goals::new()),
            Some(false)
        );
        let goals: Goals = [
            (
                "task:t-done".to_string(),
                GoalLiveness::Closed("task:t-done is done".into()),
            ),
            ("task:t-open".to_string(), GoalLiveness::Open),
        ]
        .into_iter()
        .collect();
        assert_eq!(loads_nowhere(&done, Some(&keys), &goals), Some(true));
        assert_eq!(
            loads_nowhere(&done, None, &goals),
            Some(true),
            "closed needs no walk of the records"
        );
        assert_eq!(loads_nowhere(&open, Some(&keys), &goals), Some(false));
        let tallies = BTreeMap::new();
        let line = describe(&done, &tallies, Some(&keys), &goals);
        assert!(
            line.contains("LOADS NOWHERE: task:t-done is done"),
            "{line}"
        );
        assert!(!describe(&open, &tallies, Some(&keys), &goals).contains("LOADS NOWHERE"));
        let mut retired = done.clone();
        retired.retired_at = Some("2026-09-20T00:00:00Z".into());
        assert_eq!(loads_nowhere(&retired, Some(&keys), &goals), Some(false));
        // The board could not be read: unknown, said in the prose, never
        // "loads".
        let unread: Goals = [(
            "task:t-done".to_string(),
            GoalLiveness::Unknown("the board could not be read: no graph server".into()),
        )]
        .into_iter()
        .collect();
        assert_eq!(loads_nowhere(&done, Some(&keys), &unread), None);
        assert!(describe(&done, &tallies, Some(&keys), &unread).contains("is unknown"));
        // A records walk that already says nowhere still says so.
        let elsewhere = Rule {
            scope: Some(
                Situation::of_run(&["shell".into()], Some(&PathBuf::from("/gone")))
                    .toward(task("task:t-done"))
                    .scope(),
            ),
            ..done.clone()
        };
        assert_eq!(loads_nowhere(&elsewhere, Some(&keys), &unread), Some(true));
    }

    /// A scope naming a workspace, surface or goal loads only where some run
    /// record presented those keys; one naming none of them is presented by
    /// construction; the roster says LOADS NOWHERE for an active rule the
    /// records never presented, and nothing when the store could not be
    /// read.
    #[test]
    fn a_scope_no_run_record_presents_is_said_to_load_nowhere() {
        use mecha_core::situation::Situation;
        let w = PathBuf::from("/w");
        let on_tui = Situation::of_run(&["shell".into()], Some(&w)).on(Some(SessionKind::Tui));
        let morning = || Some(GoalKey::Named("trigger:morning".parse().unwrap()));
        let keys: Presented = vec![
            (Some(w.clone()), Some(SessionKind::Tui), None),
            (None, Some(SessionKind::Slack), None),
            (Some(w.clone()), Some(SessionKind::Trigger), morning()),
        ];
        // The goal is presented the same way: toward a goal some record
        // was matched toward, with the other keys it names, and never a
        // goal no record presented or one this build cannot name.
        let toward = |g: Option<GoalKey>| {
            Situation::of_run(&["shell".into()], Some(&w))
                .on(Some(SessionKind::Trigger))
                .toward(g)
                .scope()
        };
        assert!(presented(&toward(morning()), &keys));
        assert!(presented(
            &Situation::default().toward(morning()).scope(),
            &keys
        ));
        assert!(!presented(
            &toward(Some(GoalKey::Named("trigger:evening".parse().unwrap()))),
            &keys
        ));
        assert!(
            !presented(
                &Situation::of_run(&["shell".into()], Some(&w))
                    .on(Some(SessionKind::Tui))
                    .toward(morning())
                    .scope(),
                &keys
            ),
            "the goal and the surface must be presented together"
        );
        let parked_goal = Rule {
            text: "Parked goal.".into(),
            id: Some("r-pg".into()),
            scope: Some(toward(Some(GoalKey::Unread("dream:x".into())))),
            ..Default::default()
        };
        assert_eq!(loads_nowhere(&parked_goal, None, &Goals::new()), Some(true));
        assert_eq!(
            loads_nowhere(&parked_goal, Some(&keys), &Goals::new()),
            Some(true)
        );
        assert!(!needs_presented_keys(&[&parked_goal]));
        let gone_goal = Rule {
            text: "Gone goal.".into(),
            id: Some("r-gg".into()),
            scope: Some(toward(Some(GoalKey::Named("task:done".parse().unwrap())))),
            ..Default::default()
        };
        assert_eq!(
            loads_nowhere(&gone_goal, Some(&keys), &Goals::new()),
            Some(true)
        );
        assert!(needs_presented_keys(&[&gone_goal]));
        assert!(presented(&on_tui.scope(), &keys));
        assert!(
            !presented(
                &Situation::of_run(&["shell".into()], Some(&w))
                    .on(Some(SessionKind::Slack))
                    .scope(),
                &keys
            ),
            "the workspace and the surface must be presented together"
        );
        assert!(!presented(
            &Situation::of_run(&["shell".into()], Some(&PathBuf::from("/elsewhere"))).scope(),
            &keys
        ));
        assert!(
            presented(&Situation::of_run(&["shell".into()], None).scope(), &keys),
            "no such key: presented by construction"
        );
        assert!(presented(&Situation::default(), &Presented::new()));
        let mut dark = Rule {
            text: "Dark.".into(),
            id: Some("r-dark".into()),
            scope: Some(
                Situation::of_run(&["shell".into()], Some(&PathBuf::from("/gone"))).scope(),
            ),
            ..Default::default()
        };
        let tallies = BTreeMap::new();
        assert!(describe(&dark, &tallies, Some(&keys), &Goals::new()).contains("LOADS NOWHERE"));
        assert!(
            !describe(&dark, &tallies, None, &Goals::new()).contains("LOADS NOWHERE"),
            "unknown is not nowhere"
        );
        dark.retired_at = Some("2026-09-07T00:00:00Z".into());
        assert!(
            !describe(&dark, &tallies, Some(&keys), &Goals::new()).contains("LOADS NOWHERE"),
            "a retired rule loads nowhere by design"
        );
        assert_eq!(
            loads_nowhere(&dark, Some(&keys), &Goals::new()),
            Some(false),
            "retired: not flagged"
        );
        dark.retired_at = None;
        assert_eq!(loads_nowhere(&dark, Some(&keys), &Goals::new()), Some(true));
        assert_eq!(
            loads_nowhere(&dark, None, &Goals::new()),
            None,
            "unknown store: unknown"
        );
        let tools_only = Rule {
            scope: Some(Situation::of_run(&["shell".into()], None).scope()),
            ..dark.clone()
        };
        assert_eq!(
            loads_nowhere(&tools_only, None, &Goals::new()),
            Some(false),
            "by construction, whatever the store"
        );
        assert_eq!(
            loads_nowhere(&Rule::default(), None, &Goals::new()),
            None,
            "unscoped: no claim"
        );
        // A parked surface provably matches no run: nowhere, whatever the
        // store holds or whether it could be read.
        let parked = Rule {
            scope: Some(Situation {
                surface_unread: Some("copilot".into()),
                ..Situation::of_run(&["shell".into()], None)
            }),
            ..dark.clone()
        };
        assert_eq!(loads_nowhere(&parked, None, &Goals::new()), Some(true));
        assert_eq!(
            loads_nowhere(&parked, Some(&keys), &Goals::new()),
            Some(true)
        );
        assert!(!presented(&parked.scope.clone().unwrap().scope(), &keys));
        // A corpus-mark surface on the stored scope: stripped by scope(),
        // refused by matches — nowhere, and the roster says so.
        let marked = Rule {
            scope: Some(Situation::of_run(&["shell".into()], None).on(Some(SessionKind::Test))),
            ..dark.clone()
        };
        assert_eq!(loads_nowhere(&marked, None, &Goals::new()), Some(true));
        assert!(describe(&marked, &tallies, Some(&keys), &Goals::new()).contains("LOADS NOWHERE"));
        // With no tools at all the same scope is standing by definition,
        // and the prose must still say nowhere, as the JSON does.
        let bare_mark = Rule {
            scope: Some(Situation::default().on(Some(SessionKind::Test))),
            ..dark.clone()
        };
        assert_eq!(loads_nowhere(&bare_mark, None, &Goals::new()), Some(true));
        let line = describe(&bare_mark, &tallies, None, &Goals::new());
        assert!(line.contains("LOADS NOWHERE"), "{line}");
        assert!(!line.contains("standing"), "{line}");
    }

    /// The store is read from the top of each transcript only, and a torn
    /// one makes the whole answer unknown: two hand-written transcripts —
    /// one whose first run record names a workspace and surface, one with
    /// no header — and the walk answers the pair for the first alone, then
    /// `None` once the second is there. Fails on `Session::list`, which
    /// drops the torn file silently and answers with the pair.
    #[test]
    fn presented_keys_come_off_every_run_record_and_a_torn_store_is_unknown() {
        use mecha_core::session::{Record, RunConfig, SessionMeta};
        use mecha_core::situation::Situation;
        let dir = std::env::temp_dir().join(format!(
            "mecha-presented-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let s = Session::create(
            &dir,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                workspace: PathBuf::from("/jail"),
                title: None,
                kind: Some(SessionKind::Task),
            },
        )
        .unwrap();
        s.append(&Record::Config(RunConfig {
            rules_workspace: Some(PathBuf::from("/w")),
            rules_surface: Some(SessionKind::Web),
            ..Default::default()
        }))
        .unwrap();
        // A second run record later in the file names another pair — a
        // resumed run's — and it counts: a rule can be minted from it.
        s.append(&Record::Config(RunConfig {
            rules_workspace: Some(PathBuf::from("/later")),
            ..Default::default()
        }))
        .unwrap();
        let keys = presented_keys_in(&dir, &[]).expect("a readable store answers");
        assert_eq!(
            keys,
            vec![
                (Some(PathBuf::from("/w")), Some(SessionKind::Web), None),
                (Some(PathBuf::from("/later")), None, None),
            ]
        );
        // A torn trailing line is a killed process's residue and is
        // tolerated; a torn line with records after it is not.
        let residue = dir.join("residue");
        let r = Session::create(
            &residue,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                workspace: PathBuf::from("/jail"),
                title: None,
                kind: Some(SessionKind::Tui),
            },
        )
        .unwrap();
        r.append(&Record::Config(RunConfig {
            rules_workspace: Some(PathBuf::from("/w")),
            ..Default::default()
        }))
        .unwrap();
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&r.path)
                .unwrap();
            // A torn message line in the middle is never parsed and never
            // hides a run record; a torn *run record* trailing the file is
            // a killed process's residue and is tolerated.
            f.write_all(b"{\"record\":\"message\",\"trunc\n{\"record\":\"config\",\"trunc")
                .unwrap();
        }
        assert!(
            presented_keys_in(&residue, &[]).is_some(),
            "trailing residue tolerated"
        );
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&r.path)
                .unwrap();
            f.write_all(b"\n").unwrap();
        }
        r.append(&Record::Config(RunConfig::default())).unwrap();
        assert_eq!(
            presented_keys_in(&residue, &[]),
            None,
            "torn in the middle: unknown"
        );
        // The walk stops once every wanted pair has been seen, newest
        // first: an older transcript torn in the middle is never opened
        // when the newest already presents the pair. Fails on a walk that
        // reads the whole store (it would answer unknown).
        let stopped = dir.join("stopped");
        let older = Session::create(
            &stopped,
            SessionMeta {
                id: "20260101T000000-00000000".into(),
                created_at: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                workspace: PathBuf::from("/jail"),
                title: None,
                kind: Some(SessionKind::Tui),
            },
        )
        .unwrap();
        older
            .append(&Record::Config(RunConfig {
                rules_workspace: Some(PathBuf::from("/old")),
                ..Default::default()
            }))
            .unwrap();
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&older.path)
                .unwrap();
            f.write_all(b"{\"record\":\"config\",\"trunc\n").unwrap();
        }
        older.append(&Record::Config(RunConfig::default())).unwrap();
        let newer = Session::create(
            &stopped,
            SessionMeta {
                id: "20260901T000000-ffffffff".into(),
                created_at: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                workspace: PathBuf::from("/jail"),
                title: None,
                kind: Some(SessionKind::Tui),
            },
        )
        .unwrap();
        newer
            .append(&Record::Config(RunConfig {
                rules_workspace: Some(PathBuf::from("/w")),
                rules_surface: Some(SessionKind::Tui),
                ..Default::default()
            }))
            .unwrap();
        let wanted = vec![(Some(PathBuf::from("/w")), Some(SessionKind::Tui), None)];
        assert_eq!(
            presented_keys_in(&stopped, &wanted),
            Some(wanted.clone()),
            "answered by the newest transcript; the torn older one never read"
        );
        assert_eq!(
            presented_keys_in(&stopped, &[]),
            None,
            "the full walk reaches the torn record and is unknown"
        );
        // A retired rule scoped to a workspace nothing presents must not
        // force the full walk on the live rule's behalf: filtered out of
        // both gates, the live rule is answered by the newest transcript
        // and the torn older one is never opened. Fails on gates built
        // from every rule, which answered the live rule `null`.
        let live = Rule {
            id: Some("r-live".into()),
            text: "Live.".into(),
            scope: Some(
                Situation::of_run(&["shell".into()], Some(Path::new("/w")))
                    .on(Some(SessionKind::Tui)),
            ),
            ..Default::default()
        };
        let mut retired = Rule {
            id: Some("r-dead".into()),
            text: "Dead.".into(),
            scope: Some(Situation::of_run(
                &["shell".into()],
                Some(Path::new("/gone")),
            )),
            ..Default::default()
        };
        retired.retired_at = Some("2026-09-01T00:00:00Z".into());
        assert!(needs_keys(&live));
        assert!(!needs_keys(&retired));
        let keys = presented_keys_from(&[&retired, &live], &stopped)
            .expect("answered by the newest transcript");
        assert_eq!(
            loads_nowhere(&live, Some(&keys), &Goals::new()),
            Some(false)
        );
        let marked_only = Rule {
            scope: Some(Situation::of_run(&["shell".into()], None).on(Some(SessionKind::Test))),
            ..live.clone()
        };
        assert!(!needs_keys(&marked_only), "answered without the store");
        assert_eq!(
            presented_keys_from(&[&marked_only], &stopped),
            None,
            "no walk at all"
        );
        // A store with no record carrying either key cannot answer: a
        // directory that does not exist, and one holding only a record
        // from before the fields, are both unknown — never "nowhere".
        assert_eq!(presented_keys_in(&dir.join("no-such-dir"), &[]), None);
        let old_only = dir.join("old-only");
        let o = Session::create(
            &old_only,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                workspace: PathBuf::from("/jail"),
                title: None,
                kind: Some(SessionKind::Tui),
            },
        )
        .unwrap();
        o.append(&Record::Config(RunConfig::default())).unwrap();
        assert_eq!(
            presented_keys_in(&old_only, &[]),
            None,
            "pre-field records are not evidence"
        );
        // A transcript with no header: the store can no longer be read in
        // full, and the answer is unknown rather than a pair set with a
        // hole in it.
        std::fs::write(dir.join("torn.jsonl"), "{\"type\":\"message\",\"truncated").unwrap();
        assert_eq!(presented_keys_in(&dir, &[]), None);
        std::fs::remove_dir_all(&dir).ok();
        // And the walk is not made at all for a store whose rules name no
        // such key.
        let tools_only = Rule {
            scope: Some(Situation::of_run(&["shell".into()], None).scope()),
            ..Default::default()
        };
        assert!(!needs_presented_keys(&[&tools_only]));
        let scoped = Rule {
            scope: Some(Situation::of_run(&["shell".into()], Some(Path::new("/w"))).scope()),
            ..Default::default()
        };
        assert!(needs_presented_keys(&[&tools_only, &scoped]));
    }

    /// Staged rather than applied, a narrowing is a proposal whose rule
    /// carries the new scope, and a pending twin is not re-staged.
    #[test]
    fn a_narrowing_stages_as_a_proposal_and_is_not_staged_twice() {
        use mecha_core::situation::Situation;
        let sit = |tools: &[&str]| {
            Situation::of_run(
                &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                None,
            )
        };
        let store = temp_store();
        store
            .write_learned_rules(
                "behavior",
                &[Rule {
                    scope: Some(Situation::default()),
                    support: vec![sit(&["shell"]), sit(&["http_fetch"])],
                    ..rule("Widened rule.", "r-wide")
                }],
            )
            .unwrap();
        for i in 0..3 {
            store
                .append_validation(&ValidationRecord {
                    region: Some(sit(&["shell"])),
                    ..regression("r-wide", &format!("2026-09-0{}T00:00:00Z", i + 1))
                })
                .unwrap();
        }
        // A pending *learn* proposal for the domain carries the rule
        // forward unchanged — with a `narrowed_at` from an earlier scan —
        // and proposes nothing about it: not a twin, and the scan must
        // still stage. Fails on the flag-based twin test (found on review).
        // ... and one that *widened* it is a scope change the other way,
        // not this scan's narrowing (found on the fourth review pass).
        let carried = store.learned_rules("behavior").unwrap();
        let mut was_narrower = carried.clone();
        was_narrower[0].scope = Some(sit(&["fs_read"]));
        store
            .write_proposal(&mecha_core::learning::Proposal {
                id: "learn-pending".into(),
                domain: "behavior".into(),
                status: "pending".into(),
                reflexion_ids: vec!["refl-claimed".into()],
                rules_before: was_narrower,
                rules: carried.clone(),
                evidence: String::new(),
                created_at: "2026-09-02T00:00:00Z".into(),
                resolved_at: None,
                reason: None,
                scope: None,
            })
            .unwrap();
        propose(&store, 3, false, None).unwrap();
        propose(&store, 3, false, None).unwrap();
        let proposals = store.proposals().unwrap();
        assert_eq!(
            proposals.len(),
            2,
            "the learn proposal is not a twin; the narrowing stages once"
        );
        assert!(
            proposals.iter().all(|p| p.status == "pending"),
            "the learn proposal is left alone"
        );
        let proposals: Vec<_> = proposals
            .into_iter()
            .filter(|p| p.id != "learn-pending")
            .collect();
        assert_eq!(proposals.len(), 1, "a pending twin is not re-staged");
        let staged = proposals[0]
            .rules
            .iter()
            .find(|r| r.id.as_deref() == Some("r-wide"))
            .unwrap();
        assert!(staged.active());
        assert_eq!(staged.scope, Some(sit(&["http_fetch"])));
        assert!(staged.narrowed_at.is_some());
        assert!(proposals[0]
            .evidence
            .contains("narrowed: loads with http_fetch"));
        // The live rule is untouched until someone accepts.
        let live = &store.learned_rules("behavior").unwrap()[0];
        assert_eq!(live.scope, Some(Situation::default()));
        std::fs::remove_dir_all(store.root()).ok();
    }

    /// **The D1 leash is reachable: a probationary rule convicts at 2, and
    /// an ordinary rule with the same evidence survives.** Fails on the old
    /// release predicate (`observations > 0`): the two conviction rows were
    /// themselves observations, so probation was stripped in the same scan
    /// that read them and the rule answered to the ordinary threshold of 3 —
    /// `PROBATION_RETIRE_AT` could never be the operative threshold, and the
    /// only brake in front of an ungraded rule was two-thirds longer than
    /// every document said.
    #[test]
    fn a_probationary_rule_convicts_at_the_shorter_leash() {
        let store = temp_store();
        let mut bad = rule("Bad ungraded rule.", "r-probation");
        bad.probation = true;
        store
            .write_learned_rules("behavior", &[bad, rule("Plain rule.", "r-plain")])
            .unwrap();
        // Two attributed regressions each — conviction evidence and nothing
        // else, so the probationary rule's graded history is exactly its
        // convictions and the leash must hold.
        for i in 0..2 {
            store
                .append_validation(&regression(
                    "r-probation",
                    &format!("2026-08-3{}T00:00:00Z", i),
                ))
                .unwrap();
            store
                .append_validation(&regression("r-plain", &format!("2026-08-3{}T12:00:00Z", i)))
                .unwrap();
        }

        propose(&store, mecha_core::learning::DEFAULT_RETIRE_AT, true, None).unwrap();

        let live = store.learned_rules("behavior").unwrap();
        let bad = live
            .iter()
            .find(|r| r.id.as_deref() == Some("r-probation"))
            .unwrap();
        assert!(
            !bad.active(),
            "two attributed regressions retire a probationary rule"
        );
        assert!(
            bad.retired_reason.as_deref().unwrap().contains("probation"),
            "the record names the shorter leash, not the ordinary threshold: {:?}",
            bad.retired_reason
        );
        assert!(
            live.iter()
                .find(|r| r.id.as_deref() == Some("r-plain"))
                .unwrap()
                .active(),
            "the same evidence leaves an ordinary rule below its threshold"
        );

        std::fs::remove_dir_all(store.root()).ok();
    }

    /// Row 2e-5b in the scan (R41): the owner's verdicts, when their Wilson
    /// bound tenures a probationary rule, give it the ordinary leash — the
    /// same two convictions no longer retire it — and a record under the
    /// minimum does nothing, however clean. Retirement still runs on the
    /// ledger: a tenured rule with three convictions retires.
    #[test]
    fn the_owners_tenure_lifts_the_probation_leash_and_nothing_more() {
        use mecha_core::tenure::{OwnerRecord, Tally};
        let owner = |accepted, rejected| {
            let mut t = Tally::default();
            for id in ["r-tenured", "r-thrice"] {
                t.records.insert(
                    id.into(),
                    OwnerRecord {
                        accepted,
                        rejected,
                        unread: 0,
                    },
                );
            }
            t
        };
        for (tally, survives) in [(owner(41, 10), true), (owner(19, 0), false)] {
            let store = temp_store();
            let mut bad = rule("Owner-vouched ungraded rule.", "r-tenured");
            bad.probation = true;
            let mut thrice = rule("Owner-vouched but convicted thrice.", "r-thrice");
            thrice.probation = true;
            store
                .write_learned_rules("behavior", &[bad, thrice])
                .unwrap();
            for i in 0..3 {
                if i < 2 {
                    store
                        .append_validation(&regression(
                            "r-tenured",
                            &format!("2026-08-3{i}T00:00:00Z"),
                        ))
                        .unwrap();
                }
                store
                    .append_validation(&regression("r-thrice", &format!("2026-08-2{i}T00:00:00Z")))
                    .unwrap();
            }
            propose(
                &store,
                mecha_core::learning::DEFAULT_RETIRE_AT,
                true,
                Some(&tally),
            )
            .unwrap();
            let live = store.learned_rules("behavior").unwrap();
            let find = |id: &str| live.iter().find(|r| r.id.as_deref() == Some(id)).unwrap();
            assert_eq!(
                find("r-tenured").active(),
                survives,
                "tenured keeps the ordinary leash; 19 of 19 is under the minimum"
            );
            assert!(
                find("r-tenured").probation,
                "the leash is the scan's: the file keeps the mark after --apply writes"
            );
            assert!(
                !find("r-thrice").active(),
                "retirement stays on measured regressions, tenure or not"
            );
            std::fs::remove_dir_all(store.root()).ok();
        }
    }

    /// The roster says where each rule stands with the owner and whether
    /// its region is quiet — and the quiet rule still reads active: row
    /// 2e-5c reports, it never evicts.
    #[test]
    fn the_roster_line_names_tenure_and_a_quiet_region() {
        use mecha_core::situation::Situation;
        use mecha_core::tenure::{OwnerRecord, Tally};
        let now = chrono::Utc::now();
        let mut tally = Tally::default();
        tally.records.insert(
            "r-lakeside".into(),
            OwnerRecord {
                accepted: 41,
                rejected: 10,
                unread: 0,
            },
        );
        let mut recurrence = Recurrence::default();
        recurrence.runs = vec![Situation::of_run(&["mail_send".to_string()], None)];
        let standing = Standing {
            tally,
            recurrence: Some(recurrence),
            now,
        };
        let mut r = rule("Cite the Lakeside figures.", "r-lakeside");
        r.probation = true;
        r.created_at = Some((now - chrono::Duration::days(60)).to_rfc3339());
        r.scope = Some(Situation::of_run(&["http_fetch".to_string()], None).scope());
        let line = standing_line(&r, &standing);
        assert!(line.contains("tenured: 41 of 51"), "{line}");
        assert!(line.contains("the ordinary leash"), "{line}");
        assert!(line.contains("QUIET"), "{line}");
        assert!(r.active(), "reported, never evicted");
        let other = rule("Keep drafts short for sam@example.edu.", "r-other");
        let line = standing_line(&other, &standing);
        assert!(line.contains("not enough owner verdicts: 0 of 0"), "{line}");
    }

    #[test]
    fn retire_and_restore_round_trip_by_id_prefix() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("Rule one.", "r-20260805-aaaa")])
            .unwrap();
        retire(&store, "r-20260805", Some("measured harmful".into())).unwrap();
        let r = &store.learned_rules("behavior").unwrap()[0];
        assert!(!r.active());
        assert_eq!(r.retired_reason.as_deref(), Some("measured harmful"));

        // Already-retired is an error, not a silent double-stamp.
        assert!(retire(&store, "r-20260805", None).is_err());

        restore(&store, "r-20260805").unwrap();
        let r = &store.learned_rules("behavior").unwrap()[0];
        assert!(r.active() && r.retired_at.is_none() && r.retired_reason.is_none());
        std::fs::remove_dir_all(store.root()).ok();
    }

    #[test]
    fn an_ambiguous_or_unknown_rule_id_is_an_error() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("A.", "r-1a"), rule("B.", "r-1b")])
            .unwrap();
        assert!(find_rule(&store, "r-1").is_err(), "prefix matches two");
        assert!(find_rule(&store, "r-9").is_err(), "matches none");
        assert!(find_rule(&store, "r-1a").is_ok());
        std::fs::remove_dir_all(store.root()).ok();
    }

    /// `rid.starts_with("")` is true for every id, so an empty needle used to
    /// resolve to whichever learned rule happened to be alone in its domain
    /// — a TUI row with no id (a user rule, or a pre-identity learned one,
    /// both of which serialise `"id": null` and read back as `""`) would
    /// silently retire an unrelated rule instead of failing. This is the
    /// case with exactly one learned rule on disk, where the old code found
    /// exactly one hit and acted on it.
    #[test]
    fn an_empty_id_never_matches_a_rule_by_accident() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("Only rule.", "r-only")])
            .unwrap();
        assert!(
            find_rule(&store, "").is_err(),
            "an empty needle matches nothing, not everything"
        );
        assert!(retire(&store, "", None).is_err());
        // The rule is untouched.
        let r = &store.learned_rules("behavior").unwrap()[0];
        assert!(r.active());
        std::fs::remove_dir_all(store.root()).ok();
    }

    #[test]
    fn show_prints_the_rule_and_its_tally() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("Ship the thing.", "r-show")])
            .unwrap();
        assert!(show(&store, "r-show", &Goals::new(), None).is_ok());
        assert!(
            show(&store, "", &Goals::new(), None).is_err(),
            "no id given"
        );
        assert!(
            show(&store, "nope", &Goals::new(), None).is_err(),
            "no such rule"
        );
        std::fs::remove_dir_all(store.root()).ok();
    }
}
