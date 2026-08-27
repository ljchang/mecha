//! `mecha sessions` — look at what past runs actually did.

use crate::GlobalOpts;
use anyhow::Result;
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

pub async fn execute(_global: &GlobalOpts, args: Args) -> Result<()> {
    let dir = Session::default_dir()?;

    match args {
        Args::Health { days, limit, json } => health(&dir, days, limit, json)?,

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
