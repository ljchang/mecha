//! `mecha frontdoor` — inbound requests, and the quarantine they pass through.
//!
//! The human half of [`mecha_core::frontdoor`]. Three verbs, and the split
//! between them is the quarantine itself:
//!
//! - `list` and `show` are for **you**. `show` prints the prose, because a
//!   person reading a stranger's request in a terminal is the safe context —
//!   you cannot be prompt-injected into sending your own calendar somewhere.
//! - `extract` is the quarantined pass: a tool-less model call per record,
//!   turning prose into typed fields. Nothing it produces has any authority; it
//!   is the *only* representation of the prose a privileged run will ever see.
//! - `next` is what a triage trigger runs. It prints exactly what
//!   `Record::for_privileged_run` allows and nothing else, so the thing feeding
//!   a run with calendar and mail access cannot accidentally include the words
//!   a stranger typed.
//!
//! Draining is deliberately not here: `factory-publish drain` speaks the
//! protocol and holds the key, and the common case — nothing new — must cost
//! zero tokens and no model at all.

use anyhow::Result;
use mecha_core::frontdoor::{extract, Frontdoor, Record};

use crate::GlobalOpts;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// What has arrived, and what state each request is in (default).
    List {
        /// Only this state: `drained`, `extracted`, `extraction_failed`, …
        #[arg(long)]
        state: Option<String>,
    },
    /// One request in full, **including the prose a stranger wrote**.
    ///
    /// This is the one place the original text is printed, and a terminal is
    /// where it is safe: reading it costs nothing, and nothing here can act.
    Show { seq: i64 },
    /// Run the quarantined extraction over everything not yet extracted.
    Extract {
        /// Just this one.
        #[arg(long)]
        seq: Option<i64>,
        /// Re-extract records that already have an extraction.
        #[arg(long)]
        force: bool,
    },
    /// Print what a triage run may be told, as JSON — extractions only, never
    /// prose. This is what a trigger pipes into a prompt.
    Next {
        /// At most this many.
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = Frontdoor::open_default()?;
    match args.cmd.unwrap_or(Cmd::List { state: None }) {
        Cmd::List { state } => list(&store, state.as_deref()),
        Cmd::Show { seq } => show(&store, seq),
        Cmd::Extract { seq, force } => extract_all(global, &store, seq, force).await,
        Cmd::Next { limit } => next(&store, limit),
    }
}

fn list(store: &Frontdoor, state: Option<&str>) -> Result<()> {
    let records = store.records()?;
    let shown: Vec<&Record> = records
        .iter()
        .filter(|r| state.is_none_or(|s| r.state == s))
        .collect();

    if shown.is_empty() {
        println!(
            "nothing waiting in {} — `factory-publish drain` fetches what the box holds",
            store.root().display()
        );
        return Ok(());
    }
    for record in &shown {
        let flag = if !record.valid {
            "  INVALID"
        } else if record
            .extraction
            .as_ref()
            .is_some_and(|e| e.reads_like_instructions)
        {
            "  ⚠ reads like instructions"
        } else {
            ""
        };
        println!(
            "{:<5} {:<14} {:<18} {}{}",
            record.seq,
            record.type_id,
            record.state,
            record
                .extraction
                .as_ref()
                .map(|e| e.topic.clone())
                .unwrap_or_else(|| "—".into()),
            flag
        );
    }
    Ok(())
}

fn show(store: &Frontdoor, seq: i64) -> Result<()> {
    let record = store.record(seq)?;
    println!(
        "request {} · {} · {}",
        record.seq, record.type_id, record.state
    );
    println!("received {}", record.created_at);
    println!("drained  {}", record.drained_at);
    if !record.valid {
        println!(
            "\nINVALID: {}",
            record.invalid_reason.as_deref().unwrap_or("(no reason)")
        );
    }

    println!("\nfields the form validated:");
    for (name, value) in record.typed_values() {
        println!("  {name:<22} {value}");
    }

    match &record.extraction {
        Some(e) => {
            println!("\nextraction (what a triage run is allowed to see):");
            println!("  topic                  {}", e.topic);
            println!("  urgency_claimed        {}", e.urgency_claimed);
            println!("  institution            {}", e.institution);
            println!("  dates_mentioned        {}", e.dates_mentioned.join(", "));
            println!("\n  reading: {}", e.reading);
            if e.reads_like_instructions {
                // A label, never a gate. It is shown loudly because a person is
                // about to read the prose underneath it.
                println!(
                    "\n  ⚠ the extractor thinks this text tries to instruct its reader.\n\
                     \x20   That is a label on a record you are reading, not a block: the\n\
                     \x20   detection literature is clear that gating on it rejects real\n\
                     \x20   people and still passes the attack that mattered."
                );
            }
        }
        None => println!(
            "\nnot extracted{}",
            record
                .extraction_error
                .as_ref()
                .map(|e| format!(" — {e}"))
                .unwrap_or_default()
        ),
    }

    let prose = record.prose();
    if !prose.is_empty() {
        println!("\n─── what they wrote ─────────────────────────────────────────");
        println!("(their words, printed for you and for nothing with tools)\n");
        for (name, text) in prose {
            println!("{name}:\n{text}\n");
        }
    }
    Ok(())
}

async fn extract_all(
    global: &GlobalOpts,
    store: &Frontdoor,
    seq: Option<i64>,
    force: bool,
) -> Result<()> {
    // A provider and nothing else — no registry, no workspace, no approver.
    // The extractor is a bare model call by construction, and building an agent
    // here would mean the quarantine had a tool surface to be talked into using.
    let cwd = std::env::current_dir()?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| provider.default_model().to_string());
    eprintln!("extracting with {model} ({provider_name})");

    let records: Vec<Record> = store
        .records()?
        .into_iter()
        .filter(|r| seq.is_none_or(|s| r.seq == s))
        // An invalid record is never extracted: it did not validate against the
        // manifest, so nothing about it is known to be the shape it claims.
        .filter(|r| r.valid)
        .filter(|r| force || r.extraction.is_none())
        .collect();

    if records.is_empty() {
        println!("nothing to extract");
        return Ok(());
    }

    let (mut done, mut failed) = (0usize, 0usize);
    for mut record in records {
        // A record with no prose needs no extractor, and paying a model call to
        // read an empty string would be the polling mistake one layer down.
        if record.prose().is_empty() {
            record.extraction = Some(Default::default());
            record.state = "extracted".into();
            store.write(&record)?;
            done += 1;
            println!("{:<5} no prose — nothing to quarantine", record.seq);
            continue;
        }

        match extract(provider.as_ref(), &model, &record).await {
            Ok(extraction) => {
                let flagged = extraction.reads_like_instructions;
                let topic = extraction.topic.clone();
                record.extraction = Some(extraction);
                record.extraction_error = None;
                record.state = "extracted".into();
                store.write(&record)?;
                done += 1;
                println!(
                    "{:<5} {}{}",
                    record.seq,
                    topic,
                    if flagged {
                        "   ⚠ reads like instructions"
                    } else {
                        ""
                    }
                );
            }
            // Never a pass-through. The record stops here and waits for a
            // person; handing on unextracted prose is the one behaviour that
            // would make this layer decorative.
            Err(e) => {
                record.extraction_error = Some(format!("{e:#}"));
                record.state = "extraction_failed".into();
                store.write(&record)?;
                failed += 1;
                eprintln!("{:<5} extraction failed: {e:#}", record.seq);
            }
        }
    }
    println!("\n{done} extracted, {failed} failed and waiting for you");
    if failed > 0 {
        println!("read them with `mecha frontdoor show <seq>`");
    }
    Ok(())
}

fn next(store: &Frontdoor, limit: usize) -> Result<()> {
    let handed: Vec<serde_json::Value> = store
        .records()?
        .iter()
        .filter(|r| r.state == "extracted")
        .filter_map(|r| r.for_privileged_run())
        .take(limit)
        .collect();
    println!("{}", serde_json::to_string_pretty(&handed)?);
    Ok(())
}
