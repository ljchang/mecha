//! `mecha validate` — measure whether the learned rules actually help.
//!
//! The counterfactual probe: for each followup-triggered reflection, rebuild
//! the recorded conversation up to the moment the user stepped in, then ask
//! the probe question twice — once with the current rules injected, once
//! without — and have a judge grade both answers against what the
//! intervention showed the user wanted. A rule set that does not move these
//! verdicts on reflections it did not train on is prompt clutter.
//!
//! Scope, honestly: followups only. A steer or denial lands mid-run, between
//! a tool call and its result, and probing there needs the replay driver to
//! carry the run — that is the rumination milestone, not this one. And the
//! verdicts are judge-graded, so treat a single flip as a prompt to read the
//! two answers, not as a result — the same rule the eval rig already lives
//! by.

use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::config::Config;
use mecha_core::eval::Judge;
use mecha_core::learning::{locate_followup, strip_rules_block, LearningStore, Trigger};
use mecha_core::message::{CompletionRequest, Message};
use mecha_core::session::Session;

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
    /// held-out set, once `mecha learn --holdout` exists to leave one.
    #[arg(long)]
    pub unprocessed_only: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    let rules_block = store.rules_prompt_block()?;
    let Some(rules_block) = rules_block else {
        println!("no rules to validate — run `mecha learn` first");
        return Ok(());
    };

    let reflexions: Vec<_> = store
        .reflexions()?
        .into_iter()
        .filter(|r| r.trigger == Trigger::Followup.as_str())
        .filter(|r| !args.unprocessed_only || !r.is_processed)
        .collect();
    if reflexions.is_empty() {
        println!("no followup reflections to probe");
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

    let sessions_dir = Session::default_dir()?;
    let mut improved = 0u32;
    let mut regressed = 0u32;
    let mut unchanged = 0u32;
    let mut skipped = 0u32;

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
        let Some(idx) = locate_followup(&convo.messages, &r.intervention) else {
            eprintln!("· {}: could not locate the intervention turn; skipping", r.id);
            skipped += 1;
            continue;
        };

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

        let label = match (baseline, with) {
            (false, true) => {
                improved += 1;
                "IMPROVED"
            }
            (true, false) => {
                regressed += 1;
                "REGRESSED"
            }
            _ => {
                unchanged += 1;
                if baseline { "unchanged (both pass)" } else { "unchanged (both fail)" }
            }
        };
        println!("· {} [{}] {}", r.id, label, r.reflexion_text);
        if baseline != with {
            println!("    baseline:   {}", first_line(&answers[0]));
            println!("    with rules: {}", first_line(&answers[1]));
        }
    }

    println!(
        "\n{improved} improved, {regressed} regressed, {unchanged} unchanged, {skipped} skipped \
         (n={}, judge-graded — read the answers before believing a single flip)",
        reflexions.len()
    );
    Ok(())
}

fn first_line(s: &str) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if line.chars().count() > 140 {
        format!("{}…", line.chars().take(140).collect::<String>())
    } else {
        line.to_string()
    }
}
