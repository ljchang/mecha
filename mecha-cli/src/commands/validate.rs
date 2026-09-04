//! `mecha validate` — measure whether the learned rules actually help.
//!
//! Every trigger gets a counterfactual probe, and the two kinds are graded
//! differently on purpose:
//!
//! - **Followups** re-ask the user's corrective turn against the rebuilt
//!   conversation, with and without the rules, and a judge grades both
//!   answers. Judge-graded, so a single flip is a prompt to read the two
//!   answers, not a result.
//! - **Steers and denials** land mid-run, so their probes *branch* the
//!   recording at the intervention — the recorded messages before it are
//!   resubmitted verbatim, steering text stripped, and the model generates
//!   only the continuation, which makes the replay the no-steer
//!   counterfactual by construction and pre-point divergence impossible —
//!   once per arm, and the verdict is structural: did the model do the
//!   steered thing without the steer, did it repeat the exact call the user
//!   refused. Trace-graded, no judge. See [`mecha_core::counterfactual`]
//!   for the verdict semantics.
//!
//! A rule set that does not move these verdicts on reflections it did not
//! train on is prompt clutter, and `mecha learn --holdout` exists to keep
//! such reflections available.
//!
//! Every probe's outcome is appended to the store's **validation ledger**
//! (`validations.jsonl`), keyed to the exact rule set measured — printed
//! numbers are a report, the ledger is evidence, and `mecha rules` folds it
//! into per-rule tallies. When a trace-graded probe *regresses* (passes
//! rules-free, fails with the rules), the probe **bisects** the active
//! learned rules against the same recorded prefix to name the rule that
//! flips it, and the attribution lands in the ledger too. Bisection assumes
//! a single culprit; a regression that needs several rules together, or one
//! the user's own rules cause, is recorded unattributed — never guessed.

use crate::{probe, setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::config::{Config, ProviderConfig};
use mecha_core::counterfactual::ProbeVerdict;
use mecha_core::eval::Judge;
use mecha_core::learning::{
    domain_rules_section_for, locate_followup, rules_hash, strip_rules_block, wrap_rules_block,
    LearningStore, Origin, Reflexion, Rule, Trigger, ValidationRecord,
};
use mecha_core::message::{CompletionRequest, Message};
use mecha_core::session::Session;
use mecha_core::situation::Situation;
use std::collections::BTreeMap;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Provider entry the judge runs on. Defaults to the model under test,
    /// which is worth avoiding for the usual reason.
    #[arg(long)]
    pub judge_provider: Option<String>,

    /// Judge model id.
    #[arg(long)]
    pub judge_model: Option<String>,

    /// Only validate reflections not yet consumed by a learn pass — the
    /// held-out set `mecha learn --holdout` leaves.
    #[arg(long)]
    pub unprocessed_only: bool,

    /// Probe only these triggers (comma-separated: steer, denial, followup).
    /// Default is all three.
    #[arg(long, value_delimiter = ',')]
    pub trigger: Vec<String>,

    /// Skip the bisection that attributes a regression to one rule.
    /// Regressions are still recorded in the ledger, just unattributed.
    #[arg(long)]
    pub no_attribute: bool,
}

/// The active learned rules as a flat list, with what a bisection needs to
/// rebuild a candidate block from any subset of them.
struct RuleSurface {
    /// `(domain, rule)`, in domain order — the ledger's `rule_ids` and the
    /// bisection's index space.
    flat: Vec<(String, Rule)>,
    user_by_domain: BTreeMap<String, Vec<Rule>>,
}

impl RuleSurface {
    /// Loads the **run domains only** (`RUN_DOMAINS`). A probe replays a
    /// tool-having run, so its block must be one such a run can carry: the
    /// `triage` domain rides in the mail classifier's tool-less pass and
    /// nowhere else, and `Reflexion::learnable`'s untrusted-origin exemption
    /// for it is argued from exactly that — a triage rule in a probe prompt
    /// would put mail-derived text in front of an agent with tools, and a
    /// ledger row naming its id would charge observations to a rule the
    /// measured run could never have had (found on review).
    fn load(store: &LearningStore) -> Result<Self> {
        let mut flat = Vec::new();
        let mut user_by_domain = BTreeMap::new();
        for domain in mecha_core::learning::RUN_DOMAINS {
            let domain = domain.to_string();
            user_by_domain.insert(domain.clone(), store.user_rules(&domain)?);
            for rule in store.learned_rules(&domain)? {
                if rule.active() {
                    flat.push((domain.clone(), rule));
                }
            }
        }
        Ok(RuleSurface {
            flat,
            user_by_domain,
        })
    }

    /// Indices into `flat` of the rules a run in `run`'s situation carries
    /// — the measured block for a probe over that run, and the bisection's
    /// space. A rule scoped to a tool the run never registered is not in
    /// the block, so it cannot be observed or convicted there.
    fn carried(&self, run: &Situation) -> Vec<usize> {
        self.flat
            .iter()
            .enumerate()
            .filter(|(_, (_, r))| r.scope.as_ref().is_none_or(|s| s.matches(run)))
            .map(|(i, _)| i)
            .collect()
    }

    /// Ids of the selected rules — what a ledger row charges its observation
    /// to. Rules from before identity existed have none; they ride, but no
    /// tally can accumulate against them.
    fn rule_ids(&self, selected: &[usize]) -> Vec<String> {
        selected
            .iter()
            .filter_map(|i| self.flat[*i].1.id.clone())
            .collect()
    }

    /// Render the block a run in `run`'s situation would see if only the
    /// selected learned rules (by index into `flat`) existed. User rules
    /// are not on trial and an arm without them would measure a deployment
    /// that cannot exist — but they ride under the same scope filter the
    /// run path applies (`domain_rules_section_for`), because a hand-scoped
    /// user rule a real run in this situation drops must not ride in the
    /// measured arm either (found on review).
    fn block_with(&self, selected: &[usize], run: &Situation) -> Option<String> {
        let mut sections = Vec::new();
        for (domain, user) in &self.user_by_domain {
            let learned: Vec<Rule> = self
                .flat
                .iter()
                .enumerate()
                .filter(|(i, (d, _))| d == domain && selected.contains(i))
                .map(|(_, (_, r))| r.clone())
                .collect();
            sections.extend(domain_rules_section_for(domain, user, &learned, run));
        }
        wrap_rules_block(sections)
    }
}

/// Bisect a confirmed regression down to one learned rule.
///
/// Precondition: rules-free passed, the full block failed. Every test drives
/// the same recorded prefix under a block holding a subset of the learned
/// rules; if the probe still fails, the culprit is inside the subset. Returns
/// the index of the single rule that flips the verdict, or `None` when no
/// single rule does — the user's rules alone regress it, several rules only
/// fail together, or an arm came back inconclusive. `None` is an honest
/// answer: attribution must never be a guess, because retirement argues from
/// it.
async fn attribute_regression(
    prepared: &setup::Prepared,
    provider_cfg: &ProviderConfig,
    model: &str,
    prep: &probe::ProbePrep,
    surface: &RuleSurface,
    carried: &[usize],
) -> Result<Option<usize>> {
    if carried.is_empty() {
        return Ok(None);
    }
    let fails = |selected: Vec<usize>| async move {
        let block = surface.block_with(&selected, &prep.situation());
        match probe::drive_arm(
            prepared,
            provider_cfg,
            model,
            prep,
            prep.system_with(block.as_deref()),
        )
        .await?
        {
            Ok(ProbeVerdict::Fail) => Ok(Some(true)),
            Ok(ProbeVerdict::Pass) => Ok(Some(false)),
            // An inconclusive or failed arm aborts the whole attribution.
            Ok(ProbeVerdict::Inconclusive(_)) | Err(_) => Ok::<_, anyhow::Error>(None),
        }
    };

    // If the user's own rules already regress this probe, no learned rule can
    // be charged with it — a final single-rule test would blame whichever
    // rule happened to ride beside them.
    match fails(Vec::new()).await? {
        Some(false) => {}
        _ => return Ok(None),
    }

    // The full block is a confirmed failure, so the culprit is in the full
    // set; each round keeps whichever half still fails.
    let mut set: Vec<usize> = carried.to_vec();
    while set.len() > 1 {
        let (a, b) = set.split_at(set.len() / 2);
        let (a, b) = (a.to_vec(), b.to_vec());
        match fails(a.clone()).await? {
            Some(true) => {
                set = a;
                continue;
            }
            Some(false) => {}
            None => return Ok(None),
        }
        match fails(b.clone()).await? {
            Some(true) => set = b,
            // Neither half fails alone: the regression needs rules from both.
            _ => return Ok(None),
        }
    }
    Ok(set.first().copied())
}

/// Which reflections this run is willing to probe.
///
/// Trigger and processed-state are the caller's own filters; the third is
/// not optional. **A `Derived` reflection is mecha's own voice read back as
/// a user's correction** — `Reflexion::provenance()` is `Derived` exactly
/// when `agent::is_harness_voice` recognises the intervention text, which
/// means the "steer" `counterfactual::locate_steer` would find in the
/// transcript is a harness nudge or a peer's mailbox delivery, not something
/// anyone typed to correct a mistake. Probing it still produces a Pass/Fail
/// verdict, and a `Fail` there runs `attribute`, which bisects the active
/// rules against that same recorded prefix to name a culprit — so a probe
/// point with no ground truth in it can charge a real rule with a
/// regression, and three of those retire the rule from every future prompt.
///
/// `learnable()` is the wrong gate for this: its triage exemption lets some
/// `Untrusted` reflections through for a reason specific to consolidation
/// (untrusted *content* policy in the triage domain), which is a different
/// question from whether this particular "steer" is genuine. Filtering on
/// that would drop real evidence this probe should keep, so the check here
/// is narrower and asks only the question that matters to a probe.
fn select_probe_corpus(
    reflexions: Vec<Reflexion>,
    wanted_triggers: &[&str],
    unprocessed_only: bool,
) -> Vec<Reflexion> {
    reflexions
        .into_iter()
        .filter(|r| wanted_triggers.contains(&r.trigger.as_str()))
        .filter(|r| !unprocessed_only || !r.is_processed)
        // Dropped is the owner saying no, and it outranks measurement the
        // same way it outranks candidacy (`learnable()`'s own first clause):
        // a probe over a refused reflection still writes ledger rows, and a
        // regression there can bisect to — and help retire — a real rule on
        // evidence the owner already rejected.
        .filter(|r| r.dropped_at.is_none())
        .filter(|r| r.provenance() != Origin::Derived)
        .collect()
}

/// Whether an answer can be graded at all.
///
/// **An answer that is not there is not a failing answer**, and the
/// difference decides whether a rule gets blamed. The followup probe re-asks
/// the corrective turn with `tools: Vec::new()` — no tool surface — so a run
/// that would naturally have continued by calling one produces either nothing
/// or a bare `<tool_call>` residue where the text should be. Handed to the
/// judge that reads as a bad answer, so the arm "fails"; when only the
/// with-rules arm does it, the comparison reports REGRESSED and the rule is
/// convicted for an artifact of the harness.
///
/// Two of the three regressions in the first full pass over this store
/// (2026-08-29) were exactly that. It mattered the moment retirement stopped
/// needing a human: a false regression now retires a rule on its own.
///
/// So an ungradeable arm makes the probe *inconclusive* — the same answer
/// this file already gives a torn transcript or an absent session. The
/// alternative is not "grade it anyway", it is a ledger that cannot be
/// trusted by the thing that reads it.
fn is_gradeable(answer: &str) -> bool {
    let t = answer.trim();
    if t.is_empty() {
        return false;
    }
    // Tool-call markup with no prose *outside* it. Hermes-style `<tool_call>`
    // is what qwen emits here, and the whole span goes, payload included: a
    // call's JSON body is not an answer, and stripping only the delimiters
    // let `<tool_call>{"name":"fs_read",…}</tool_call>` reach the judge as
    // one — the manufactured-regression path this function exists to close,
    // with retirement now automatic behind it. An unclosed opening tag (a
    // stop-string truncation, the shape observed live) swallows to the end
    // for the same reason: everything after it is call, not prose.
    let stripped = strip_spans(
        &strip_spans(t, "<tool_call>", "</tool_call>"),
        "<tool_response>",
        "</tool_response>",
    );
    !stripped.trim().is_empty()
}

/// Remove every `open`…`close` span, the markers and everything between
/// them; an unclosed `open` removes through the end of the string.
fn strip_spans(s: &str, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(open) {
        out.push_str(&rest[..i]);
        rest = &rest[i + open.len()..];
        match rest.find(close) {
            Some(j) => rest = &rest[j + close.len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    // What a run actually carries, not what the store holds: the ledger is
    // keyed to the rule set measured, so measuring a set no run has makes
    // every attribution point at the wrong thing.
    // The store's view answers only "is there anything to measure"; the
    // block each probe measures is rendered for the situation of the session
    // it replays, below, since a scoped rule rides only where its tool is.
    if store
        .rules_prompt_block_for(mecha_core::learning::RUN_DOMAINS)?
        .is_none()
    {
        println!("no rules to validate — run `mecha learn` first");
        return Ok(());
    }

    let wanted_triggers: Vec<&str> = if args.trigger.is_empty() {
        vec![
            Trigger::Steer.as_str(),
            Trigger::Denial.as_str(),
            Trigger::Followup.as_str(),
        ]
    } else {
        args.trigger.iter().map(String::as_str).collect()
    };
    let reflexions: Vec<_> =
        select_probe_corpus(store.reflexions()?, &wanted_triggers, args.unprocessed_only);
    if reflexions.is_empty() {
        println!("no reflections to probe");
        return Ok(());
    }

    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| provider.default_model().to_string());

    let (judge_name, judge_cfg) = cfg.provider(args.judge_provider.as_deref())?;
    let judge = Judge::new(
        mecha_core::provider::build(judge_cfg)?,
        args.judge_model.clone().or_else(|| judge_cfg.model.clone()),
    )
    // The override is gone: `Judge::new` now defaults to
    // `provider::LOCAL_MAX_TOKENS`, which is above what this call site raised
    // it to. The lesson this comment recorded — that 4096 was measured
    // insufficient on the very first probe — is why that constant exists, and
    // keeping a second number here is how the fix stayed applied in one place
    // and not the others for as long as it did.
    ;
    eprintln!(
        "probing {} reflection(s) with {model} ({provider_name}), judged by {} ({judge_name})",
        reflexions.len(),
        judge.model()
    );

    // Prove the judge answers before grading anything, and only when this
    // pass will actually use it — a steer/denial-only run replays and never
    // judges, so demanding a judge there would refuse work that needs none.
    if reflexions.iter().any(|r| r.trigger == "followup") {
        judge.preflight().await?;
    }

    // Steer and denial probes replay against the recorded tool surface, which
    // needs the live registry for specs — builtins, MCP servers, subagents.
    // Built once, only when something will use it; the parent agent it builds
    // is discarded and only its registry is borrowed, as in `mecha replay`.
    let needs_replay = reflexions
        .iter()
        .any(|r| r.trigger != Trigger::Followup.as_str());
    let prepared = if needs_replay {
        Some(setup::prepare(&global.clone(), false).await?)
    } else {
        None
    };

    let sessions_dir = Session::default_dir()?;
    let mut improved = 0u32;
    let mut regressed = 0u32;
    let mut unchanged = 0u32;
    let mut inconclusive = 0u32;
    let mut skipped = 0u32;
    let mut recorded_rows = 0u32;

    // What every ledger row this run charges its observation to. Loaded once:
    // the block was rendered from this same state, and a mid-run rules change
    // would make rows describe a set that was never measured.
    let surface = RuleSurface::load(&store)?;
    let mut record = |r: &mecha_core::learning::Reflexion,
                      carried: &[usize],
                      run: &Situation,
                      outcome: &str,
                      attributed: Option<String>|
     -> Result<()> {
        // Keyed to the block *this* probe carried: with scoped loading the
        // measured set is a function of the replayed run's situation, and a
        // row that named the store's whole set would charge observations to
        // rules that were not in the prompt.
        let block = surface.block_with(carried, run).unwrap_or_default();
        // Append-only, no store lock: a validate run must never block the
        // reflect a closing session fires, and a single appended line needs
        // no read-modify-write.
        store.append_validation(&ValidationRecord {
            reflexion_id: r.id.clone(),
            trigger: r.trigger.clone(),
            domain: r.domain.clone(),
            rules_hash: rules_hash(&block),
            rule_ids: surface.rule_ids(carried),
            outcome: outcome.into(),
            attributed_rule_id: attributed,
            model: model.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })?;
        recorded_rows += 1;
        Ok(())
    };

    for r in &reflexions {
        let path = match Session::find(&sessions_dir, &r.session_id) {
            Ok(p) => p,
            Err(_) => {
                eprintln!("· {}: session {} not found; skipping", r.id, r.session_id);
                skipped += 1;
                continue;
            }
        };
        let (_, convo) = Session::load(&path)?;

        // The recorded system prompt, with any rules block of its era removed:
        // the baseline arm must be rules-free and the treatment arm must carry
        // exactly the current rules, not a mixture of generations.
        // ── steers and denials: replay the prefix, grade the trace ──
        if r.trigger != Trigger::Followup.as_str() {
            let prepared = prepared.as_ref().expect("built because needs_replay");
            let prep = match probe::prepare_probe(&sessions_dir, r)? {
                Ok(prep) => prep,
                Err(why) => {
                    eprintln!("· {}: {why}; skipping", r.id);
                    skipped += 1;
                    continue;
                }
            };
            // Rendered for the probe's own recorded config, which is the
            // one the branch replays under (a later attach may differ).
            let carried = surface.carried(&prep.situation());
            let Some(rules_block) = surface.block_with(&carried, &prep.situation()) else {
                eprintln!(
                    "· {}: no rule rides in the recorded run's situation; skipping",
                    r.id
                );
                skipped += 1;
                continue;
            };
            let mut arms = Vec::new();
            for block in [None, Some(rules_block.as_str())] {
                match probe::drive_arm(
                    prepared,
                    provider_cfg,
                    &model,
                    &prep,
                    prep.system_with(block),
                )
                .await?
                {
                    Ok(v) => arms.push(v),
                    Err(why) => {
                        eprintln!("· {}: {why}; skipping", r.id);
                        break;
                    }
                }
            }
            let [baseline, with] = &arms[..] else {
                skipped += 1;
                continue;
            };
            match probe::compare(
                baseline,
                with,
                &mut improved,
                &mut regressed,
                &mut unchanged,
                &mut inconclusive,
            ) {
                Some(label) => {
                    println!("· {} [{}, {label}] {}", r.id, r.trigger, r.reflexion_text)
                }
                None => {
                    let why = [baseline, with]
                        .iter()
                        .find_map(|v| match v {
                            ProbeVerdict::Inconclusive(w) => Some(w.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    println!("· {} [{}] inconclusive: {why}", r.id, r.trigger);
                }
            }

            // A regression is the suspicion attribution acts on: find which
            // rule flips this probe, against the same recorded prefix.
            let mut attributed = None;
            if matches!((baseline, with), (ProbeVerdict::Pass, ProbeVerdict::Fail))
                && !args.no_attribute
            {
                match attribute_regression(
                    prepared,
                    provider_cfg,
                    &model,
                    &prep,
                    &surface,
                    &carried,
                )
                .await?
                {
                    Some(i) => {
                        let (domain, rule) = &surface.flat[i];
                        match &rule.id {
                            Some(id) => {
                                attributed = Some(id.clone());
                                println!("    attributed to [{domain}] {}", rule.text);
                            }
                            // A rule from before identity existed can be
                            // named but not tallied — the next learn pass
                            // mints its id.
                            None => println!(
                                "    attributed to a pre-identity rule [{domain}]: {}",
                                rule.text
                            ),
                        }
                    }
                    None => println!("    no single rule attributable"),
                }
            }
            record(
                r,
                &carried,
                &prep.situation(),
                outcome_str(baseline, with),
                attributed,
            )?;
            continue;
        }

        // ── followups: re-ask the corrective turn, judge both answers ──
        let first_config = Session::run_configs(&path)?.into_iter().next();
        let base_system = first_config
            .as_ref()
            .and_then(|rc| rc.system_prompt.clone())
            .map(|s| strip_rules_block(&s))
            .unwrap_or_default();
        // The situation the recorded run was in, from its own record. A
        // session with no config recorded cannot say, and gets the standing
        // rules only — the set every run carries — rather than a guess.
        let run = first_config
            .as_ref()
            .map(|rc| Situation::of_run(&rc.tools, Some(&rc.workspace)))
            .unwrap_or_default();
        // The situation a followup is judged in is the session's first
        // config's — the judge path re-asks the corrective turn under the
        // recorded prompt and has no branch to replay. The steer/denial
        // path above rendered its own block for `prep.situation()`, the
        // config covering the intervention; an earlier draft gated both
        // paths on this one and skipped replays for a reason drawn from a
        // situation the replay never used (found on review).
        let carried = surface.carried(&run);
        // A block that renders nothing is nothing to measure: both arms
        // would be byte-identical and grade as "unchanged", which the
        // summary counts as a verdict — the measured-clean-versus-not-
        // measured conflation one function over (found on review). The
        // predicate is the rendered block, not `carried`: user rules ride
        // in every block regardless of selection, so a domain with user
        // rules and nothing scoped to this run still has an arm to measure
        // (found on the next review). The row is not written.
        let Some(rules_block) = surface.block_with(&carried, &run) else {
            eprintln!(
                "· {}: no rule rides in this session's situation; skipping",
                r.id
            );
            skipped += 1;
            continue;
        };
        let with_rules = if base_system.is_empty() {
            rules_block.clone()
        } else {
            format!("{base_system}\n\n{rules_block}")
        };

        let Some(idx) = locate_followup(&convo.messages, &r.intervention) else {
            eprintln!(
                "· {}: could not locate the intervention turn; skipping",
                r.id
            );
            skipped += 1;
            continue;
        };

        let mut messages: Vec<Message> = convo.messages[..idx].to_vec();
        messages.push(Message::user(r.intervention.clone()));

        let mut answers = Vec::new();
        for system in [&base_system, &with_rules] {
            let request = CompletionRequest {
                model: model.clone(),
                system: (!system.is_empty()).then(|| system.clone()),
                messages: messages.clone(),
                tools: Vec::new(),
                // **Above the reasoning budget, not merely large.** A local
                // model with thinking on spends this allowance on
                // `reasoning_content` first and emits the answer from what is
                // left; at 4096 it routinely spent all of it and returned
                // HTTP 200 with empty content — `finish_reason: "length"`,
                // 10k+ reasoning characters, no answer. The judge then graded
                // an absent answer as a bad one, and where only one arm did it
                // the probe reported REGRESSED and blamed the rule.
                //
                // The same number, for the same reason, as the judge's own
                // `with_max_tokens` twenty lines up — whose comment already
                // records that 4096 was measured insufficient. The fix landed
                // on the judge and not on the probe it grades, which is how a
                // measured lesson stays half-applied.
                max_tokens: mecha_core::provider::LOCAL_MAX_TOKENS,
                effort: None,
                thinking: false,
                cache_prompt: true,
            };
            let response = provider.complete(&request, None).await?;
            answers.push(response.message.text());
        }

        // The rubric is what the intervention itself established the user
        // wanted; the reflection's lesson names it directly.
        let rubric = format!(
            "the answer does what the user's message asks, in the light of this \
             known expectation: {}",
            r.reflexion_text
        );
        // Before the judge sees them: an arm with no readable answer cannot
        // be compared, and pretending otherwise convicts a rule of a harness
        // artifact.
        if let Some(i) = answers.iter().position(|a| !is_gradeable(a)) {
            eprintln!(
                "· {}: the {} answer has no gradeable text (a tool call with no prose, \
                 most likely — the followup probe offers no tools); inconclusive",
                r.id,
                if i == 0 { "baseline" } else { "with-rules" }
            );
            record(r, &carried, &run, "inconclusive", None)?;
            inconclusive += 1;
            continue;
        }

        let mut verdicts = Vec::new();
        for answer in &answers {
            match judge.assess(&r.intervention, &rubric, answer).await {
                Ok(v) => verdicts.push(v.pass),
                Err(e) => {
                    eprintln!("· {}: judge failed ({e:#}); skipping", r.id);
                    verdicts.clear();
                    break;
                }
            }
        }
        let [baseline, with] = verdicts[..] else {
            skipped += 1;
            continue;
        };

        let (label, outcome) = match (baseline, with) {
            (false, true) => {
                improved += 1;
                ("IMPROVED", "improved")
            }
            (true, false) => {
                regressed += 1;
                ("REGRESSED", "regressed")
            }
            (true, true) => {
                unchanged += 1;
                ("unchanged (both pass)", "unchanged_pass")
            }
            (false, false) => {
                unchanged += 1;
                ("unchanged (both fail)", "unchanged_fail")
            }
        };
        println!("· {} [{}] {}", r.id, label, r.reflexion_text);
        if baseline != with {
            println!("    baseline:   {}", first_line(&answers[0]));
            println!("    with rules: {}", first_line(&answers[1]));
        }
        // Judge-graded, so no bisection: a followup regression is a prompt to
        // read two answers, not evidence that convicts one rule.
        record(r, &carried, &run, outcome, None)?;
    }

    println!(
        "\n{improved} improved, {regressed} regressed, {unchanged} unchanged, \
         {inconclusive} inconclusive, {skipped} skipped (n={}; steers and denials are \
         trace-graded, followups judge-graded — read before believing a single flip)",
        reflexions.len()
    );
    if recorded_rows > 0 {
        println!(
            "{recorded_rows} row(s) appended to the validation ledger — `mecha rules` folds them"
        );
        store.commit(&format!("validate: {recorded_rows} probe(s) → ledger"));
    }
    Ok(())
}

/// The ledger's outcome vocabulary for a trace-graded probe pair.
fn outcome_str(baseline: &ProbeVerdict, with: &ProbeVerdict) -> &'static str {
    match (baseline, with) {
        (ProbeVerdict::Inconclusive(_), _) | (_, ProbeVerdict::Inconclusive(_)) => "inconclusive",
        (ProbeVerdict::Fail, ProbeVerdict::Pass) => "improved",
        (ProbeVerdict::Pass, ProbeVerdict::Fail) => "regressed",
        (ProbeVerdict::Pass, _) => "unchanged_pass",
        (ProbeVerdict::Fail, _) => "unchanged_fail",
    }
}

fn first_line(s: &str) -> String {
    let line = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > 140 {
        format!("{}…", line.chars().take(140).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .join("mecha-validate-test")
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

    fn rule(text: &str, id: Option<&str>) -> Rule {
        Rule {
            text: text.into(),
            id: id.map(Into::into),
            ..Default::default()
        }
    }

    fn reflexion(intervention: &str, origin: Origin) -> Reflexion {
        Reflexion {
            id: "r1".into(),
            domain: "behavior".into(),
            session_id: "s".into(),
            trigger: Trigger::Steer.as_str().into(),
            context: "c".into(),
            intervention: intervention.into(),
            reflexion_text: "t".into(),
            error_type: None,
            confidence: None,
            is_processed: false,
            leap_run_id: None,
            created_at: "2026-08-19T00:00:00Z".into(),
            origin,
            evidence: mecha_core::learning::Evidence::Full,
            edited_at: None,
            dropped_at: None,
            dropped_reason: None,
            situation: None,
        }
    }

    /// The fourth reader this PR was named for: a reflection stored `clean`
    /// before `is_harness_voice` existed reclassifies as `Derived` through
    /// `provenance()`, and must not reach the probe corpus even though its
    /// stored `origin` still says `clean`. Fails on the pre-fix selection
    /// (trigger and processed-state alone), which would hand this reflection
    /// to a probe and let a `Fail` verdict on mecha's own empty-turn nudge
    /// bisect and attribute against a real learned rule.
    #[test]
    fn a_harness_voice_reflection_never_reaches_the_probe_corpus() {
        let real = reflexion("please use tabs, not spaces", Origin::Clean);
        let fake = reflexion(
            &format!(
                "{} `build` has now returned the same thing",
                mecha_core::boredom::NOTICE_STEM
            ),
            Origin::Clean,
        );
        let triggers = [Trigger::Steer.as_str()];

        let selected = select_probe_corpus(vec![real.clone(), fake], &triggers, false);

        assert_eq!(
            selected.len(),
            1,
            "the harness's own nudge must be excluded"
        );
        assert_eq!(selected[0].intervention, real.intervention);
    }

    /// The common case is untouched: a real steer with no harness-voice
    /// resemblance survives every filter, including the new one.
    #[test]
    fn an_ordinary_steer_still_reaches_the_probe_corpus() {
        let real = reflexion("please use tabs, not spaces", Origin::Clean);
        let triggers = [Trigger::Steer.as_str()];

        let selected = select_probe_corpus(vec![real.clone()], &triggers, false);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].intervention, real.intervention);
    }

    /// Dropped is the owner saying no, and it outranks measurement the same
    /// way it outranks candidacy: a dropped reflection must not keep being
    /// probed, writing ledger rows, and feeding bisection with evidence the
    /// owner already rejected. `learnable()` is deliberately not the gate
    /// here, so its dropped clause did not come along for free.
    #[test]
    fn a_dropped_reflection_never_reaches_the_probe_corpus() {
        let kept = reflexion("please use tabs, not spaces", Origin::Clean);
        let mut dropped = reflexion("use the old google tool", Origin::Clean);
        dropped.dropped_at = Some("2026-08-30T00:00:00Z".into());
        dropped.dropped_reason = Some("recorded surface unrecoverable".into());
        let triggers = [Trigger::Steer.as_str()];

        let selected = select_probe_corpus(vec![kept.clone(), dropped], &triggers, false);

        assert_eq!(selected.len(), 1, "the dropped reflection must be excluded");
        assert_eq!(selected[0].intervention, kept.intervention);
    }

    #[test]
    fn a_full_selection_renders_exactly_the_deployed_block() {
        // The bisection's ground assumption: block_with(everything) is the
        // very block the treatment arm measured, byte for byte — otherwise
        // the subsets it tests live in a different deployment than the
        // regression it is explaining.
        let store = temp_store();
        std::fs::write(
            store.root().join("rules/behavior.user.toml"),
            "[[rules]]\ntext = \"User rule.\"\n",
        )
        .unwrap();
        store
            .write_learned_rules(
                "behavior",
                &[
                    rule("Learned A.", Some("r-a")),
                    rule("Learned B.", Some("r-b")),
                ],
            )
            .unwrap();
        store
            .write_learned_rules("writing", &[rule("Sign off briefly.", Some("r-c"))])
            .unwrap();

        // A classifier rule never reaches a probe: the surface is the run
        // domains, not the store.
        store
            .write_learned_rules(
                mecha_core::learning::TRIAGE_DOMAIN,
                &[rule("Newsletters are ignore.", Some("r-triage"))],
            )
            .unwrap();

        let surface = RuleSurface::load(&store).unwrap();
        assert_eq!(surface.flat.len(), 3);
        assert!(surface
            .flat
            .iter()
            .all(|(d, _)| d != mecha_core::learning::TRIAGE_DOMAIN));
        let all: Vec<usize> = (0..surface.flat.len()).collect();
        assert_eq!(surface.rule_ids(&all), vec!["r-a", "r-b", "r-c"]);
        // Unscoped rules are carried by every run.
        assert_eq!(surface.carried(&Situation::default()), all);

        assert_eq!(
            surface.block_with(&all, &Situation::default()).unwrap(),
            store
                .rules_prompt_block_for(mecha_core::learning::RUN_DOMAINS)
                .unwrap()
                .unwrap()
        );

        // An empty selection still carries the user's rules — they are not on
        // trial — and none of the learned ones.
        let none = surface.block_with(&[], &Situation::default()).unwrap();
        assert!(none.contains("User rule."));
        assert!(!none.contains("Learned A.") && !none.contains("Sign off"));

        // A subset carries exactly its members.
        let one = surface.block_with(&[1], &Situation::default()).unwrap();
        assert!(one.contains("Learned B.") && !one.contains("Learned A."));

        // A scoped rule is carried only by a run that registers its tool,
        // and a ledger row for the other run must not name it.
        let mut scoped = rule("Confirm before rm -rf.", Some("r-shell"));
        scoped.scope = Some(Situation::of_run(&["shell".into()], None));
        store
            .write_learned_rules(
                "behavior",
                &[
                    rule("Ask before deleting.", Some("r-a")),
                    rule("Say what you did.", Some("r-b")),
                    scoped,
                ],
            )
            .unwrap();
        let surface = RuleSurface::load(&store).unwrap();
        let no_shell = surface.carried(&Situation::of_run(&["fs_read".into()], None));
        assert_eq!(surface.rule_ids(&no_shell), vec!["r-a", "r-b", "r-c"]);
        let with_shell = surface.carried(&Situation::of_run(&["shell".into()], None));
        assert_eq!(
            surface.rule_ids(&with_shell),
            vec!["r-a", "r-b", "r-shell", "r-c"]
        );
        let fs_only = Situation::of_run(&["fs_read".into()], None);
        let shell_run = Situation::of_run(&["shell".into()], None);
        assert!(!surface
            .block_with(&no_shell, &fs_only)
            .unwrap()
            .contains("rm -rf"));
        assert!(surface
            .block_with(&with_shell, &shell_run)
            .unwrap()
            .contains("rm -rf"));
        // A hand-scoped user rule follows the same filter as the run path.
        std::fs::write(
            store.root().join("rules/behavior.user.toml"),
            "[[rules]]\ntext = \"User rule.\"\n\n[[rules]]\ntext = \"Shell user rule.\"\n[rules.scope]\ntools = [\"shell\"]\n",
        )
        .unwrap();
        let surface = RuleSurface::load(&store).unwrap();
        assert!(!surface
            .block_with(&no_shell, &fs_only)
            .unwrap()
            .contains("Shell user rule."));
        assert!(surface
            .block_with(&with_shell, &shell_run)
            .unwrap()
            .contains("Shell user rule."));

        std::fs::remove_dir_all(store.root()).ok();
    }

    #[test]
    fn retired_and_preidentity_rules_shape_the_surface_correctly() {
        let store = temp_store();
        let retired = Rule {
            text: "Was harmful.".into(),
            enabled: false,
            id: Some("r-old".into()),
            retired_at: Some("2026-08-05T00:00:00Z".into()),
            ..Default::default()
        };
        // A rule from before identity existed rides in blocks (it is live!)
        // but cannot be tallied — rule_ids must skip it, not invent a key.
        store
            .write_learned_rules("behavior", &[rule("No id yet.", None), retired])
            .unwrap();
        let surface = RuleSurface::load(&store).unwrap();
        assert_eq!(
            surface.flat.len(),
            1,
            "retired rules are not on the surface"
        );
        assert!(surface.rule_ids(&[0]).is_empty());
        assert!(surface
            .block_with(&[0], &Situation::default())
            .unwrap()
            .contains("No id yet."));
        std::fs::remove_dir_all(store.root()).ok();
    }

    #[test]
    fn the_ledger_outcome_vocabulary_covers_the_verdict_grid() {
        use ProbeVerdict::*;
        let inc = || Inconclusive("why".into());
        assert_eq!(outcome_str(&Fail, &Pass), "improved");
        assert_eq!(outcome_str(&Pass, &Fail), "regressed");
        assert_eq!(outcome_str(&Pass, &Pass), "unchanged_pass");
        assert_eq!(outcome_str(&Fail, &Fail), "unchanged_fail");
        assert_eq!(outcome_str(&inc(), &Pass), "inconclusive");
        assert_eq!(outcome_str(&Pass, &inc()), "inconclusive");
    }
}

#[cfg(test)]
mod gradeable_tests {
    use super::is_gradeable;

    /// **A missing answer is not a failing answer.** The followup probe offers
    /// no tools, so a run that would have continued with a tool call comes
    /// back as bare markup — and a judge reads that as bad. When only one arm
    /// does it the probe reports REGRESSED and the rule wears it.
    ///
    /// Fails on the old behaviour: every string here went straight to the
    /// judge, and two of the three regressions in the first full pass over the
    /// live store were manufactured this way.
    #[test]
    fn tool_call_residue_and_emptiness_are_not_gradeable_answers() {
        for bad in [
            "",
            "   ",
            "\n\n",
            "<tool_call>",
            "  <tool_call>  ",
            "<tool_call></tool_call>",
            "<tool_response></tool_response>",
            // The payload survives delimiter-stripping, which is how a real
            // tool-call emission used to reach the judge as an answer: the
            // span goes whole, JSON body included.
            "<tool_call>{\"name\":\"fs_read\",\"arguments\":{\"path\":\"a.md\"}}</tool_call>",
            // An unclosed tag is a stop-string truncation mid-call; what
            // follows it is call, not prose.
            "<tool_call>{\"name\":\"fs_read\",\"argu",
            // Anything *inside* the markers is being emitted as a call, not
            // said to the user — prose-shaped or not.
            "<tool_call>Let me read the file first, then answer.</tool_call>",
        ] {
            assert!(!is_gradeable(bad), "{bad:?} has nothing a judge can read");
        }
    }

    /// Real prose grades, including prose that carries a call alongside an
    /// actual answer — the check is on what remains outside the spans, not on
    /// whether the marker appears.
    #[test]
    fn prose_grades_even_when_a_tool_call_rides_along() {
        for good in [
            "I'll populate the sheet now with the Fall 2026 dates.",
            "Got it. I'll map the same 29 meetings.",
            "Here is the answer.<tool_call></tool_call>",
            "Here is the answer.<tool_call>{\"name\":\"fs_read\"}</tool_call>",
            "no",
        ] {
            assert!(is_gradeable(good), "{good:?} is readable");
        }
    }
}
