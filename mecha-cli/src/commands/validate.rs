//! `mecha validate` — measure whether the learned rules actually help.
//!
//! Every trigger gets a counterfactual probe, and the two kinds are graded
//! differently on purpose:
//!
//! - **Followups** re-ask the user's corrective turn against the rebuilt
//!   conversation, with and without the rules, and a judge grades both
//!   answers. Judge-graded, so a single flip is a prompt to read the two
//!   answers, not a result.
//! - **Steers and denials** land mid-run, so their probes *replay* the
//!   recorded prefix — recorded tool results, seeded sampler, no steering
//!   text (extraction drops it, which makes the replay the no-steer
//!   counterfactual by construction) — once per arm, and the verdict is
//!   structural: did the model do the steered thing without the steer, did
//!   it repeat the exact call the user refused. Trace-graded, no judge. See
//!   [`mecha_core::counterfactual`] for the verdict semantics.
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
    domain_rules_section, locate_followup, rules_hash, strip_rules_block, wrap_rules_block,
    LearningStore, Rule, Trigger, ValidationRecord,
};
use mecha_core::message::{CompletionRequest, Message};
use mecha_core::session::Session;
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
    fn load(store: &LearningStore) -> Result<Self> {
        let mut flat = Vec::new();
        let mut user_by_domain = BTreeMap::new();
        for domain in store.domains() {
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

    /// Ids of the rules riding in the measured block — what a ledger row
    /// charges its observation to. Rules from before identity existed have
    /// none; they ride, but no tally can accumulate against them.
    fn rule_ids(&self) -> Vec<String> {
        self.flat.iter().filter_map(|(_, r)| r.id.clone()).collect()
    }

    /// Render the block a run would see if only the selected learned rules
    /// (by index into `flat`) existed. User rules always ride: they are not
    /// on trial, and an arm without them would measure a deployment that
    /// cannot exist.
    fn block_with(&self, selected: &[usize]) -> Option<String> {
        let mut sections = Vec::new();
        for (domain, user) in &self.user_by_domain {
            let learned: Vec<Rule> = self
                .flat
                .iter()
                .enumerate()
                .filter(|(i, (d, _))| d == domain && selected.contains(i))
                .map(|(_, (_, r))| r.clone())
                .collect();
            sections.extend(domain_rules_section(domain, user, &learned));
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
) -> Result<Option<usize>> {
    if surface.flat.is_empty() {
        return Ok(None);
    }
    let fails = |selected: Vec<usize>| async move {
        let block = surface.block_with(&selected);
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
    let mut set: Vec<usize> = (0..surface.flat.len()).collect();
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

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    // What a run actually carries, not what the store holds: the ledger is
    // keyed to the rule set measured, so measuring a set no run has makes
    // every attribution point at the wrong thing.
    let rules_block = store.rules_prompt_block_for(mecha_core::learning::RUN_DOMAINS)?;
    let Some(rules_block) = rules_block else {
        println!("no rules to validate — run `mecha learn` first");
        return Ok(());
    };

    let wanted_triggers: Vec<&str> = if args.trigger.is_empty() {
        vec![
            Trigger::Steer.as_str(),
            Trigger::Denial.as_str(),
            Trigger::Followup.as_str(),
        ]
    } else {
        args.trigger.iter().map(String::as_str).collect()
    };
    let reflexions: Vec<_> = store
        .reflexions()?
        .into_iter()
        .filter(|r| wanted_triggers.contains(&r.trigger.as_str()))
        .filter(|r| !args.unprocessed_only || !r.is_processed)
        .collect();
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
    // These rubrics carry more context than eval's, and the judge reasons
    // about the whole exchange before the JSON appears. 4096 was measured
    // insufficient on the very first probe.
    .with_max_tokens(16384);
    eprintln!(
        "probing {} reflection(s) with {model} ({provider_name}), judged by {} ({judge_name})",
        reflexions.len(),
        judge.model()
    );

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
    let block_hash = rules_hash(&rules_block);
    let ledger_rule_ids = surface.rule_ids();
    let mut record = |r: &mecha_core::learning::Reflexion,
                      outcome: &str,
                      attributed: Option<String>|
     -> Result<()> {
        // Append-only, no store lock: a validate run must never block the
        // reflect a closing session fires, and a single appended line needs
        // no read-modify-write.
        store.append_validation(&ValidationRecord {
            reflexion_id: r.id.clone(),
            trigger: r.trigger.clone(),
            domain: r.domain.clone(),
            rules_hash: block_hash.clone(),
            rule_ids: ledger_rule_ids.clone(),
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
        let base_system = Session::run_configs(&path)?
            .first()
            .and_then(|rc| rc.system_prompt.clone())
            .map(|s| strip_rules_block(&s))
            .unwrap_or_default();
        let with_rules = if base_system.is_empty() {
            rules_block.clone()
        } else {
            format!("{base_system}\n\n{rules_block}")
        };

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
                match attribute_regression(prepared, provider_cfg, &model, &prep, &surface).await? {
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
            record(r, outcome_str(baseline, with), attributed)?;
            continue;
        }

        // ── followups: re-ask the corrective turn, judge both answers ──
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
                max_tokens: 4096,
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
        record(r, outcome, None)?;
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
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir()
            .join("mecha-validate-test")
            .join(format!("{}-{nanos}", std::process::id()));
        LearningStore::open(dir).unwrap()
    }

    fn rule(text: &str, id: Option<&str>) -> Rule {
        Rule {
            text: text.into(),
            id: id.map(Into::into),
            ..Default::default()
        }
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

        let surface = RuleSurface::load(&store).unwrap();
        assert_eq!(surface.flat.len(), 3);
        assert_eq!(surface.rule_ids(), vec!["r-a", "r-b", "r-c"]);

        let all: Vec<usize> = (0..surface.flat.len()).collect();
        assert_eq!(
            surface.block_with(&all).unwrap(),
            store.rules_prompt_block().unwrap().unwrap()
        );

        // An empty selection still carries the user's rules — they are not on
        // trial — and none of the learned ones.
        let none = surface.block_with(&[]).unwrap();
        assert!(none.contains("User rule."));
        assert!(!none.contains("Learned A.") && !none.contains("Sign off"));

        // A subset carries exactly its members.
        let one = surface.block_with(&[1]).unwrap();
        assert!(one.contains("Learned B.") && !one.contains("Learned A."));

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
        assert!(surface.rule_ids().is_empty());
        assert!(surface.block_with(&[0]).unwrap().contains("No id yet."));
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
