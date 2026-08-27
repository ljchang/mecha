//! `mecha sessions` — look at what past runs actually did.

use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::message::{Block, Role};
use mecha_core::session::Session;

#[derive(clap::Subcommand, Debug)]
pub enum Args {
    /// List saved sessions, newest first.
    List {
        /// How many to show.
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
    },

    /// Print a transcript.
    Show {
        /// Session id or unique prefix.
        id: String,

        /// Emit the raw JSONL records instead of formatted text.
        #[arg(long)]
        json: bool,
    },

    /// Print the path to a session file.
    Path {
        /// Session id or unique prefix.
        id: String,
    },

    /// How past runs went, as distinct from what they cost: stop causes,
    /// tool reliability, and how often a run finished over a failure.
    Health {
        /// Only sessions started in the last N days.
        #[arg(long)]
        days: Option<i64>,

        /// Stop after this many sessions, newest first.
        #[arg(long, short = 'n')]
        limit: Option<usize>,

        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// How past runs went against what they were *for* — the signed record,
    /// and the label derived from it.
    ///
    /// Observation only: nothing consumes these, and the number worth reading
    /// is how many come back with no label at all.
    Appraise {
        /// Only sessions started in the last N days.
        #[arg(long)]
        days: Option<i64>,

        /// Stop after this many sessions, newest first.
        #[arg(long, short = 'n')]
        limit: Option<usize>,

        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,

        /// Resolve each intervention's agency by counterfactual replay.
        ///
        /// **This is the paid pass.** Without it `appraise` reads records
        /// already on disk and costs nothing; with it every intervention
        /// drives one replay of the recorded run *without* the steering text,
        /// which is a model run apiece. That is what fills `controllable` —
        /// the field 100% of the corpus's labels were stuck on.
        #[arg(long)]
        probe: bool,

        /// Ceiling on replays, across the whole walk. Newest sessions first.
        #[arg(long, default_value_t = 25, requires = "probe")]
        max_probes: usize,

        /// Run the quarantined appraiser (§5.1) over each session's evidence.
        ///
        /// **A second paid pass, independent of `--probe`.** One quarantined
        /// model call per session — no tools, no conversation, and the input
        /// is numbers only (see `AppraiserEvidence`), never the transcript.
        /// It looks for one additional signed error beyond what `of_session`
        /// already computed, or reports that the numbers support nothing
        /// further, which is the ordinary and correct answer.
        #[arg(long)]
        appraise: bool,

        /// Ceiling on appraiser calls, across the whole walk. Newest sessions
        /// first, independent of `--max-probes`.
        #[arg(long, default_value_t = 25, requires = "appraise")]
        max_appraisals: usize,
    },

    /// Total token usage — and cost, where prices are configured — across
    /// saved sessions, grouped by provider and model.
    Stats {
        /// Only sessions started in the last N days.
        #[arg(long)]
        days: Option<i64>,

        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let dir = Session::default_dir()?;

    match args {
        Args::Health { days, limit, json } => health(&dir, days, limit, json)?,

        Args::Appraise {
            days,
            limit,
            json,
            probe,
            max_probes,
            appraise: run_appraiser,
            max_appraisals,
        } => {
            appraise(
                global,
                &dir,
                days,
                limit,
                json,
                probe,
                max_probes,
                run_appraiser,
                max_appraisals,
            )
            .await?
        }

        Args::List { limit } => {
            let sessions = Session::list(&dir)?;
            if sessions.is_empty() {
                println!("no sessions in {}", dir.display());
                return Ok(());
            }
            for (meta, _) in sessions.iter().take(limit) {
                println!(
                    "{}  {}  {:<24} {}",
                    meta.id,
                    meta.created_at.format("%Y-%m-%d %H:%M"),
                    meta.model,
                    meta.title.as_deref().unwrap_or("")
                );
            }
            if sessions.len() > limit {
                println!("… {} more", sessions.len() - limit);
            }
        }

        Args::Show { id, json } => {
            let path = Session::find(&dir, &id)?;
            if json {
                print!("{}", std::fs::read_to_string(&path)?);
                return Ok(());
            }

            let (meta, convo) = Session::load(&path)?;
            println!(
                "{} · {} ({}) · {}\n",
                meta.id,
                meta.model,
                meta.provider,
                meta.created_at.format("%Y-%m-%d %H:%M:%S")
            );

            for message in &convo.messages {
                match message.role {
                    Role::User => {
                        // A user turn is either something the human typed or a
                        // batch of tool results; they read very differently.
                        let text = message.text();
                        if !text.is_empty() {
                            println!("› {text}\n");
                        }
                        for block in &message.content {
                            if let Block::ToolResult {
                                content, is_error, ..
                            } = block
                            {
                                let marker = if *is_error { "✗" } else { "✓" };
                                println!("  {marker} {}\n", first_line(content));
                            }
                        }
                    }
                    Role::Assistant => {
                        let text = message.text();
                        if !text.is_empty() {
                            println!("{text}\n");
                        }
                        for (_, name, input) in message.tool_uses() {
                            println!("  → {name} {}\n", first_line(&input.to_string()));
                        }
                    }
                }
            }
        }

        Args::Path { id } => println!("{}", Session::find(&dir, &id)?.display()),

        Args::Stats { days, json } => stats(&dir, days, json)?,
    }

    Ok(())
}

/// One row of the rollup: everything recorded under one provider+model pair.
#[derive(Default)]
struct StatRow {
    sessions: u64,
    turns: u64,
    usage: mecha_core::message::Usage,
    /// Priced at *today's* configured rates — the transcript records tokens,
    /// not prices, so historical runs are re-priced, not remembered.
    cost_usd: f64,
    priced: bool,
}

fn stats(dir: &std::path::Path, days: Option<i64>, json: bool) -> Result<()> {
    let config = mecha_core::config::Config::load(&std::env::current_dir()?)?;
    let cutoff = days.map(|d| chrono::Utc::now() - chrono::Duration::days(d));

    let mut rows = std::collections::BTreeMap::<(String, String), StatRow>::new();
    for (meta, path) in Session::list(dir)? {
        if let Some(cutoff) = cutoff {
            if meta.created_at < cutoff {
                continue;
            }
        }
        // A torn transcript still counts what it recorded; one unreadable
        // file must not sink the report.
        let (usage, turns) = Session::usage_totals(&path).unwrap_or_default();
        let pricing = config
            .providers
            .get(&meta.provider)
            .and_then(|p| p.pricing());

        let row = rows.entry((meta.provider, meta.model)).or_default();
        row.sessions += 1;
        row.turns += turns as u64;
        if let Some(pricing) = &pricing {
            row.cost_usd += usage.cost_usd(pricing);
            row.priced = true;
        }
        row.usage.add(&usage);
    }

    if json {
        let items: Vec<_> = rows
            .iter()
            .map(|((provider, model), r)| {
                serde_json::json!({
                    "provider": provider,
                    "model": model,
                    "sessions": r.sessions,
                    "turns": r.turns,
                    "input_tokens": r.usage.input_tokens,
                    "output_tokens": r.usage.output_tokens,
                    "cache_creation_input_tokens": r.usage.cache_creation_input_tokens,
                    "cache_read_input_tokens": r.usage.cache_read_input_tokens,
                    "cost_usd": r.priced.then_some(r.cost_usd),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("no sessions in {}", dir.display());
        return Ok(());
    }

    println!(
        "{:<34} {:>8} {:>7} {:>9} {:>9} {:>9} {:>9} {:>10}",
        "provider/model", "sessions", "turns", "input", "output", "cache-w", "cache-r", "cost"
    );
    let mut total = StatRow::default();
    for ((provider, model), r) in &rows {
        println!(
            "{:<34} {:>8} {:>7} {:>9} {:>9} {:>9} {:>9} {:>10}",
            format!("{provider}/{model}"),
            r.sessions,
            r.turns,
            fmt_tokens(r.usage.input_tokens),
            fmt_tokens(r.usage.output_tokens),
            fmt_tokens(r.usage.cache_creation_input_tokens),
            fmt_tokens(r.usage.cache_read_input_tokens),
            // A local model with no prices really does cost nothing; only
            // rows with a configured price claim a dollar figure.
            if r.priced {
                format!("${:.2}", r.cost_usd)
            } else {
                "—".into()
            },
        );
        total.sessions += r.sessions;
        total.turns += r.turns;
        total.usage.add(&r.usage);
        total.cost_usd += r.cost_usd;
        total.priced |= r.priced;
    }
    println!(
        "{:<34} {:>8} {:>7} {:>9} {:>9} {:>9} {:>9} {:>10}",
        "total",
        total.sessions,
        total.turns,
        fmt_tokens(total.usage.input_tokens),
        fmt_tokens(total.usage.output_tokens),
        fmt_tokens(total.usage.cache_creation_input_tokens),
        fmt_tokens(total.usage.cache_read_input_tokens),
        if total.priced {
            format!("${:.2}", total.cost_usd)
        } else {
            "—".into()
        },
    );
    if total.priced {
        println!("\ncost is at today's configured prices, not the prices at run time");
    }

    Ok(())
}

fn fmt_tokens(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=9_999_999 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.chars().count() > 120 {
        format!("{}…", line.chars().take(120).collect::<String>())
    } else {
        line.to_string()
    }
}

/// `mecha sessions appraise` — the readout rung 7 exists to produce.
///
/// **Observation only.** Nothing consumes an appraisal, and the number worth
/// reading is the neutral share: §14's own test is that if the labels come back
/// degenerate the channel is dead, and that is learned cheaply here rather than
/// after something is built on it.
///
/// Derived on the spot from the transcripts, the outbox and each run's own
/// record — see `appraisal::of_session` on why there is no store yet.
#[allow(clippy::too_many_arguments)]
async fn appraise(
    global: &GlobalOpts,
    dir: &std::path::Path,
    days: Option<i64>,
    limit: Option<usize>,
    json: bool,
    probe: bool,
    max_probes: usize,
    run_appraiser: bool,
    max_appraisals: usize,
) -> Result<()> {
    use mecha_core::appraisal;

    let since = days.map(|d| chrono::Utc::now() - chrono::Duration::days(d));

    // Best-effort, like every reader over these stores: a read failure costs
    // the `Edit` channel and nothing else. `outbox_unreadable` is kept
    // separate from an empty `drafts` deliberately — `open_existing_default`
    // returns `None` for a store that simply has never been created (a fresh
    // install, or one that has never staged a draft), which is the ordinary
    // *empty* case and not a read failure. Conflating the two prints "the
    // outbox could not be read" on a machine that has nothing to read, which
    // is the dash-versus-zero inversion this whole surface exists to avoid.
    let (drafts, outbox_unreadable): (Vec<mecha_core::outbox::OutboxItem>, bool) =
        match mecha_core::outbox::OutboxStore::open_existing_default() {
            None => (Vec::new(), false),
            Some(store) => match store.items() {
                Ok(items) => (items, false),
                Err(_) => (Vec::new(), true),
            },
        };

    // Walked here rather than through `runlog::Corpus`, and the difference is
    // the unit: that reader yields one row per **run**, which is right for
    // counting what runs cost and wrong for this — an intervention carries a
    // message index with nothing saying which run held it, and an outbox item
    // records a session. `RunStats::fold` collapses a session's runs the way
    // rung 4's episode stats do, through the same fold.
    let mut appraisals = Vec::new();
    // Kept beside each appraisal only for the probe pass. The free readout
    // never looks at them again, so this allocates nothing extra when `--probe`
    // is off — `of_session` has already read what it needs out of them.
    let mut per_session_interventions: Vec<Vec<mecha_core::learning::Intervention>> = Vec::new();
    let mut sessions_read = 0usize;
    for (meta, path) in Session::list(dir)? {
        if since.is_some_and(|t| meta.created_at < t) {
            continue;
        }
        if limit.is_some_and(|n| sessions_read >= n) {
            break;
        }
        sessions_read += 1;
        // One `read`, every reading off the same transcript: the outcome
        // (`episode_stats`'s own fold, done here in the same pass), the
        // interventions, and the goal the model's own `serves` argument
        // named. `Session::read`'s own doc names this exact pattern —
        // `episode_stats` and `load` each open and walk the file separately —
        // as the thing it exists to collapse; `Corpus::scan`-style reads with
        // no `limit` make that a real cost, not a tidiness one.
        //
        // `read` fails on an unreadable file where `episode_stats` would
        // answer `Ok(None)` for one merely lacking an outcome record, so the
        // "no outcome yet" case still has to be its own `continue` rather
        // than folding into the `Err` arm.
        let Ok(transcript) = Session::read(&path) else {
            continue;
        };
        let Some(stats) = transcript.episode else {
            continue;
        };
        let messages = transcript.convo.messages;
        let interventions = mecha_core::learning::extract_interventions(&messages);
        // Without a goal, `of_session` never has one to attribute anything
        // to — `Frustration` needs one on two negatives by construction, so
        // a session with no goal cannot produce it however it went, and the
        // neutral share this readout prints would be confusing "the channel
        // is dead" with "nothing supplied it a goal", which is a different
        // finding with a different remedy.
        let goal =
            mecha_core::tool::todo::TodoTool::plan_from_transcript(&messages).and_then(|p| p.goal);
        let goals: Vec<_> = goal.into_iter().collect();
        // The provenance gate's own rule, applied here: `stats.taint` cannot
        // tell "recorded clean" apart from "recorded before the field
        // existed", so origin is classified off the timeline's own coverage
        // instead, which answers `None` rather than guessing clean.
        let end_taint = Session::taint_timeline(&path)
            .ok()
            .and_then(|tl| tl.covering(messages.len().saturating_sub(1)));
        let mine: Vec<&mecha_core::outbox::OutboxItem> = drafts
            .iter()
            .filter(|i| i.session_id.as_deref() == Some(meta.id.as_str()))
            .collect();
        appraisals.push(appraisal::of_session(
            &meta.id,
            &stats,
            &goals,
            &interventions,
            &mine,
            end_taint,
            meta.created_at.to_rfc3339(),
        ));
        per_session_interventions.push(if probe { interventions } else { Vec::new() });
    }

    // --- The paid passes ---
    //
    // Off by default, and the free readout above is byte-for-byte what it was:
    // `appraise` with no flag still costs zero tokens and no model, which is
    // the property that lets it be run over the whole store. `--probe` and
    // `--appraise` are independent — either, neither, or both — so the
    // provider is built once for whichever is set rather than twice.
    let mut tally = crate::appraisal_probe::Tally::default();
    let mut appraiser_tally = crate::appraiser_pass::Tally::default();
    let mut budget = if probe { max_probes } else { 0 };
    let mut appraiser_budget = if run_appraiser { max_appraisals } else { 0 };
    if (probe || run_appraiser) && !appraisals.is_empty() {
        let cwd = std::env::current_dir().context("cannot determine the working directory")?;
        let cfg = mecha_core::config::Config::load(&cwd)?;
        let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
        let built = mecha_core::provider::build(provider_cfg)?;
        let model = global
            .model
            .clone()
            .or_else(|| provider_cfg.model.clone())
            .unwrap_or_else(|| built.default_model().to_string());

        if probe {
            // A replay needs the live registry for tool specs, exactly as
            // `mecha validate` and `mecha replay` do; the agent it builds is
            // discarded and only its registry is borrowed.
            let prepared = crate::setup::prepare(global, false).await?;
            let wanted: usize = per_session_interventions.iter().map(Vec::len).sum();
            // The honest ceiling, not `wanted`: `probe_appraisal` checks
            // `replayable(trigger)` before spending budget, so a `followup` or
            // an `edit` — most of an ordinary corpus — costs nothing and was
            // never going to be probed regardless of `max_probes`. Reporting
            // `wanted` here reads as a cap that will bind when it almost never
            // does.
            let replayable: usize = per_session_interventions
                .iter()
                .flatten()
                .filter(|i| crate::appraisal_probe::replayable(i.trigger))
                .count();
            eprintln!(
                "probing up to {} of {replayable} replayable intervention(s) ({wanted} total) \
                 with {model} ({provider_name})",
                max_probes.min(replayable)
            );
            for (a, interventions) in appraisals.iter_mut().zip(&per_session_interventions) {
                let t = crate::appraisal_probe::probe_appraisal(
                    &prepared,
                    provider_cfg,
                    &model,
                    dir,
                    interventions,
                    a,
                    &mut budget,
                )
                .await?;
                tally.add(t);
            }
            // **No silent caps.** The walk spends its budget newest-session-first,
            // so a truncated run describes recent work and not the corpus — which
            // is a defensible order and an indefensible thing to leave unsaid.
            // Asked of what the budget actually refused, never of the
            // intervention count: `probe_appraisal` checks `replayable(trigger)`
            // *before* spending budget, so a `followup` or an `edit` costs
            // nothing — which is the whole point of `Tally::unprobeable`
            // existing apart from `over_budget`. `wanted > max_probes` fires on a
            // corpus that is mostly followups (the common shape) even when
            // nothing was actually capped.
            if tally.over_budget > 0 {
                eprintln!(
                    "budget stopped at {max_probes}; {} replayable intervention(s) went \
                     unprobed, so the labels below describe the newest sessions, not the \
                     whole store",
                    tally.over_budget
                );
            }
        }

        if run_appraiser {
            // Run after the probe, against its own provider handle — no
            // registry needed, since the quarantined pass carries no tools.
            // Runs against the *post-probe* state: a probed intervention's
            // resolved `controllable`/`agency` is more informative evidence
            // than the pre-probe guess, so ordering the appraiser second
            // hands it the better of the two.
            eprintln!(
                "appraising up to {} of {} session(s) with {model} ({provider_name})",
                max_appraisals.min(appraisals.len()),
                appraisals.len()
            );
            for a in appraisals.iter_mut() {
                let t = crate::appraiser_pass::appraise_one(
                    built.as_ref(),
                    &model,
                    a,
                    &mut appraiser_budget,
                )
                .await?;
                appraiser_tally.add(t);
            }
            if appraiser_tally.over_budget > 0 {
                eprintln!(
                    "budget stopped at {max_appraisals}; {} session(s) went unappraised, so \
                     the labels below describe the newest sessions, not the whole store",
                    appraiser_tally.over_budget
                );
            }
        }
    }

    let mut labels: std::collections::BTreeMap<String, usize> = Default::default();
    let mut channels: std::collections::BTreeMap<String, usize> = Default::default();
    let mut positive = 0usize;
    for a in &appraisals {
        *labels.entry(enum_key(a.label)).or_default() += 1;
        for e in &a.errors {
            *channels.entry(enum_key(e.channel)).or_default() += 1;
            if e.sign > 0.0 {
                positive += 1;
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "appraised": appraisals.len(),
                "sessions_read": sessions_read,
                "labels": labels,
                "channels": channels,
                "positive_errors": positive,
                "outbox_read": !outbox_unreadable,
                // Absent, not zero, when no probe ran: "nothing was probed"
                // and "probed and found nothing" are opposite findings, and a
                // reader that cannot tell them apart is the bug this whole
                // rung exists to avoid.
                "probe": probe.then(|| serde_json::json!({
                    "driven": tally.driven,
                    "mattered": tally.mattered,
                    "redundant": tally.redundant,
                    "inconclusive": tally.inconclusive,
                    "unprobeable": tally.unprobeable,
                    "unavailable": tally.unavailable,
                    "over_budget": tally.over_budget,
                    "budget_left": budget,
                })),
                // Same "absent, not zero" rule as `probe`: whether the flag
                // ran at all is a different fact from what it found.
                "appraiser": run_appraiser.then(|| serde_json::json!({
                    "driven": appraiser_tally.driven,
                    "found_negative": appraiser_tally.found_negative,
                    "found_positive": appraiser_tally.found_positive,
                    "found_nothing": appraiser_tally.found_nothing,
                    "failed": appraiser_tally.failed,
                    "over_budget": appraiser_tally.over_budget,
                    "budget_left": appraiser_budget,
                })),
            }))?
        );
        return Ok(());
    }

    println!(
        "{} session(s) appraised, of {} read\n",
        appraisals.len(),
        sessions_read
    );
    // Printed before the early return below: a store that could not be read
    // is a fact about this run regardless of whether anything was left to
    // appraise, and the early return used to skip it whenever `appraisals`
    // came back empty — the one path where a reader most needs to know the
    // edit channel is missing rather than genuinely empty.
    if outbox_unreadable {
        println!("  (the outbox could not be read, so the edit channel is missing — not empty)\n");
    }
    if appraisals.is_empty() {
        return Ok(());
    }

    println!("  label");
    for (label, n) in &labels {
        let pct = *n as f64 / appraisals.len() as f64 * 100.0;
        println!("    {label:<16} {n:>5}  ({pct:.0}%)");
    }
    // The one that decides whether this rung goes further. Said out loud
    // rather than left to be read off the table, because it is the finding.
    let neutral = labels.get("neutral").copied().unwrap_or(0);
    println!(
        "\n  {:.0}% carry no label — six of the ten `Affect` variants need a \
         charter, a probe, a notion of harm, a cross-run view or a prediction",
        neutral as f64 / appraisals.len() as f64 * 100.0
    );

    println!("\n  signed errors, by channel");
    if channels.is_empty() {
        println!("    none");
    }
    for (channel, n) in &channels {
        println!("    {channel:<16} {n:>5}");
    }
    println!(
        "    {:<16} {:>5}  — the only channel that can say a run went well",
        "of which +ve", positive
    );

    if probe {
        println!(
            "\n  counterfactual probe ({} replay(s) driven)",
            tally.driven
        );
        println!(
            "    {:<16} {:>5}  — the steer was load-bearing: regret",
            "mattered", tally.mattered
        );
        println!(
            "    {:<16} {:>5}  — the run got there anyway: disappointment",
            "redundant", tally.redundant
        );
        // Kept apart on purpose. An inconclusive probe cost a model run and
        // posed no question; a skip cost nothing and had none to pose.
        println!(
            "    {:<16} {:>5}  — diverged before the probe point",
            "inconclusive", tally.inconclusive
        );
        // Three ways to have no finding, and they call for three different
        // responses: extend the mechanism, fix the registry, raise the budget.
        println!(
            "    {:<16} {:>5}  — followup/edit: no counterfactual to drive",
            "unprobeable", tally.unprobeable
        );
        println!(
            "    {:<16} {:>5}  — session or tool surface unavailable",
            "unavailable", tally.unavailable
        );
        println!(
            "    {:<16} {:>5}  — budget ran out first",
            "not reached", tally.over_budget
        );
    }

    if run_appraiser {
        println!(
            "\n  quarantined appraiser ({} call(s) driven)",
            appraiser_tally.driven
        );
        println!(
            "    {:<16} {:>5}  — one additional negative error added",
            "found negative", appraiser_tally.found_negative
        );
        println!(
            "    {:<16} {:>5}  — one additional positive error added",
            "found positive", appraiser_tally.found_positive
        );
        println!(
            "    {:<16} {:>5}  — the ordinary answer: nothing further",
            "found nothing", appraiser_tally.found_nothing
        );
        println!(
            "    {:<16} {:>5}  — refused, or unparseable after one retry",
            "failed", appraiser_tally.failed
        );
        println!(
            "    {:<16} {:>5}  — budget ran out first",
            "not reached", appraiser_tally.over_budget
        );
    }
    Ok(())
}

/// A `Serialize` enum's own wire spelling, never `{:?}`. Debug and serde agree
/// on every variant here today because each is one word, but `Agency::Own`
/// already diverges (`"self"`, a hand-written rename) — so deriving a JSON key
/// from Debug is a bug waiting on the first multi-word variant, silently
/// mismatched the day it lands rather than caught here. `unwrap_or_else`'s
/// fallback is unreachable for a unit-variant enum in practice; it exists so
/// this stays a display helper rather than a second thing that can panic.
fn enum_key<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

/// `sessions health` — the run-quality corpus, summarised.
///
/// Deliberately separate from `stats`, which answers what runs *cost*. This
/// answers whether they *worked*, and the two have different audiences and
/// different units. Every rate here prints `—` where its denominator is zero,
/// because no evidence is not a clean record.
fn health(
    dir: &std::path::Path,
    days: Option<i64>,
    limit: Option<usize>,
    json: bool,
) -> Result<()> {
    use mecha_core::runlog::{Corpus, Scan};

    let since = days.map(|d| chrono::Utc::now() - chrono::Duration::days(d));
    let corpus = Corpus::scan(
        dir,
        &Scan {
            max_sessions: limit,
            since,
        },
    )?;

    if json {
        println!("{}", serde_json::to_string_pretty(&as_json(&corpus))?);
        return Ok(());
    }

    if corpus.is_empty() {
        println!(
            "no recorded run outcomes in {} ({} session(s) read)",
            dir.display(),
            corpus.sessions_read
        );
        println!(
            "outcomes are recorded from the release that added the record; older\n\
         transcripts carry none"
        );
        return Ok(());
    }

    println!(
        "{} run(s) across {} session(s){}\n",
        corpus.len(),
        corpus.sessions_read,
        days.map(|d| format!(", last {d} day(s)"))
            .unwrap_or_default()
    );

    let causes: Vec<String> = corpus
        .stop_causes()
        .into_iter()
        .map(|(cause, n)| {
            let name = cause
                .map(|c| {
                    serde_json::to_string(&c)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_string()
                })
                .unwrap_or_else(|| "unrecorded".into());
            format!("{name} {n}")
        })
        .collect();
    println!("  stop cause      {}", causes.join(" · "));
    println!(
        "  ended on a failed call   {} ({})",
        corpus.ended_on_failed_call(),
        pct(corpus.rate_of(|r| r.stats.ended_on_failed_call))
    );
    println!(
        "  tool calls      {} · errors {} ({}) · denied {} · staged {}",
        corpus.tool_calls(),
        corpus.tool_errors(),
        pct(corpus.tool_error_rate()),
        corpus.rows.iter().map(|r| r.stats.tool_denied).sum::<u32>(),
        corpus.rows.iter().map(|r| r.stats.tool_staged).sum::<u32>(),
    );
    println!(
        "  malformed args {} · blocked sends {} · compactions {}",
        corpus
            .rows
            .iter()
            .map(|r| r.stats.malformed_tool_args)
            .sum::<u32>(),
        corpus
            .rows
            .iter()
            .map(|r| r.stats.blocked_sends)
            .sum::<u32>(),
        corpus.compactions(),
    );
    // Two numbers about two different things, so they get two clauses. The
    // first draft read "3 across 2 of 3 run(s) — 50.0% hit at least one",
    // where `2 of 3` is *sensor coverage* but parses as "the runs that
    // overflowed" — and then disagrees with the percentage beside it. The
    // whole reason the reader returns a pair is to keep coverage visible;
    // spending it on a phrase that reads as incidence gave the confusion back.
    let (overflows, sensed) = corpus.context_overflows();
    let hit = corpus
        .rows
        .iter()
        .filter(|r| r.stats.context_overflows.is_some_and(|n| n > 0))
        .count();
    if sensed > 0 {
        print!(
            "  context overflows   {overflows} in {hit} run(s) ({})",
            pct(corpus.overflow_rate())
        );
        // Only worth saying when the corpus is mixed; on one written entirely
        // after the sensor, the caveat is noise.
        if sensed < corpus.len() {
            print!(
                " — of {sensed} that recorded it; {} did not",
                corpus.len() - sensed
            );
        }
        println!();
    } else {
        println!("  context overflows   — (no run in this corpus recorded the counter)");
    }

    // The number every threshold in `boredom.rs` is answerable against. A dash
    // rather than a zero on a corpus with no sensor, like the line above: a
    // detector that has never fired and one that was not there yet are opposite
    // findings, and this is the field that exists to tell them apart.
    let sensed_boredom = corpus
        .rows
        .iter()
        .filter(|r| r.stats.boredom_notices.is_some())
        .count();
    let bored = corpus
        .rows
        .iter()
        .filter(|r| r.stats.boredom_notices.is_some_and(|n| n > 0))
        .count();
    if sensed_boredom > 0 {
        print!(
            "  went nowhere        {bored} run(s) told an approach had stopped moving ({})",
            pct(corpus.boredom_rate())
        );
        // Same caveat as overflows, for the same reason: worth saying only
        // when the corpus is mixed, or it reads as noise beside a clean one.
        if sensed_boredom < corpus.len() {
            print!(
                " — of {sensed_boredom} that recorded it; {} did not",
                corpus.len() - sensed_boredom
            );
        }
        println!();
    } else {
        println!("  went nowhere        — (no run in this corpus recorded the counter)");
    }

    let by_model = corpus.by_model();
    if by_model.len() > 1 {
        // A blended rate across models is true and useless: neither model
        // behaves that way, and a threshold on it fires for the wrong one.
        println!("\nby model");
        for (model, sub) in &by_model {
            println!(
                "  {:<28} {:>4} run(s)   tool errors {:>6}   ended on failure {:>6}   \
                 overflows {:>6}",
                model,
                sub.len(),
                pct(sub.tool_error_rate()),
                pct(sub.rate_of(|r| r.stats.ended_on_failed_call)),
                // The one rate here that is *more* model-bound than the
                // others: `context_window` is a per-provider setting, so a
                // corpus of a 32k local model and a wide-window cloud one has
                // no blended overflow rate worth quoting.
                pct(sub.overflow_rate()),
            );
        }
    }

    let (cost, priced) = corpus.cost_usd();
    if priced > 0 {
        println!(
            "\n${cost:.2} across {priced} of {} run(s) — a lower bound where prices are unset",
            corpus.len()
        );
    }
    Ok(())
}

fn as_json(corpus: &mecha_core::runlog::Corpus) -> serde_json::Value {
    let (cost, priced) = corpus.cost_usd();
    let (overflows, sensed) = corpus.context_overflows();
    serde_json::json!({
        "runs": corpus.len(),
        "sessions_read": corpus.sessions_read,
        "tool_calls": corpus.tool_calls(),
        "tool_errors": corpus.tool_errors(),
        "tool_error_rate": corpus.tool_error_rate(),
        "ended_on_failed_call": corpus.ended_on_failed_call(),
        "ended_on_failed_call_rate": corpus.rate_of(|r| r.stats.ended_on_failed_call),
        "compactions": corpus.compactions(),
        "context_overflows": overflows,
        "runs_with_overflow_sensor": sensed,
        "overflow_rate": corpus.overflow_rate(),
        "boredom_rate": corpus.boredom_rate(),
        "cost_usd": cost,
        "runs_priced": priced,
        "by_model": corpus
            .by_model()
            .iter()
            .map(|(model, sub)| {
                serde_json::json!({
                    "model": model,
                    "runs": sub.len(),
                    "tool_error_rate": sub.tool_error_rate(),
                    "ended_on_failed_call_rate": sub.rate_of(|r| r.stats.ended_on_failed_call),
                    "overflow_rate": sub.overflow_rate(),
                })
            })
            .collect::<Vec<_>>(),
    })
}

/// A rate as a percentage, or `—` when it has no denominator. Never `0%`:
/// "nothing went wrong" and "nothing happened" are different answers, and
/// printing them the same way is how a stopped component reads as healthy.
fn pct(rate: Option<f64>) -> String {
    match rate {
        Some(r) => format!("{:.1}%", r * 100.0),
        None => "—".into(),
    }
}
