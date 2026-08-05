//! `mecha batch` — run the same agent over many prompts.

use crate::{setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::batch::{BatchItem, BatchSummary};
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// JSONL input. Each line is `{"id": "...", "prompt": "...", "meta": {...}}`,
    /// or a bare string, which is used as both id and prompt. `-` reads stdin.
    pub input: PathBuf,

    /// Where to write results, one JSON object per line. Defaults to stdout.
    #[arg(long, short = 'o')]
    pub out: Option<PathBuf>,

    /// How many items to run at once.
    #[arg(long, short = 'c', default_value_t = 4)]
    pub concurrency: usize,

    /// Stop after this many items. Useful for a smoke test over a big file.
    #[arg(long)]
    pub limit: Option<usize>,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let mut items = read_items(&args.input)?;
    if let Some(limit) = args.limit {
        let dropped = items.len().saturating_sub(limit);
        items.truncate(limit);
        if dropped > 0 {
            eprintln!("mecha: --limit {limit} — skipping {dropped} remaining items");
        }
    }
    anyhow::ensure!(!items.is_empty(), "{} has no items", args.input.display());

    // Batch runs are unattended by definition: there is nobody to answer an
    // approval prompt, so tools that need one will be refused unless --yes.
    let prepared = setup::prepare(global, false).await?;
    if !global.yes && !global.read_only {
        eprintln!(
            "mecha: running with approvals disabled — tools that change state will be refused. \
             Pass --yes to allow them, or --read-only to make that explicit."
        );
    }

    eprintln!(
        "mecha: {} items · {} at a time · {} ({})",
        items.len(),
        args.concurrency,
        prepared.model,
        prepared.provider_name
    );

    let mut sink: Box<dyn Write + Send> = match &args.out {
        Some(path) => Box::new(std::io::BufWriter::new(
            std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?,
        )),
        None => Box::new(std::io::stdout()),
    };

    let total = items.len();
    let mut done = 0usize;
    let started = std::time::Instant::now();

    // Results are written as they finish, so a killed run still leaves
    // everything completed so far on disk.
    let results = mecha_core::batch::run(&prepared.agent, items, args.concurrency, |result| {
        done += 1;
        if let Ok(line) = serde_json::to_string(result) {
            let _ = writeln!(sink, "{line}");
            let _ = sink.flush();
        }
        let status = if result.ok { "ok " } else { "err" };
        eprintln!(
            "  [{done}/{total}] {status} {} · {} turns · {}ms",
            result.id, result.turns, result.elapsed_ms
        );
    })
    .await;

    let summary = BatchSummary::of(&results, started.elapsed().as_millis() as u64);
    eprintln!(
        "\n{}/{} succeeded · {} · {:.1}s",
        summary.succeeded,
        summary.total,
        crate::render::format_usage(&summary.usage),
        summary.elapsed_ms as f64 / 1000.0
    );

    if summary.failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn read_items(path: &PathBuf) -> Result<Vec<BatchItem>> {
    let reader: Box<dyn BufRead> = if path.as_os_str() == "-" {
        Box::new(std::io::BufReader::new(std::io::stdin()))
    } else {
        Box::new(std::io::BufReader::new(
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
        ))
    };

    let mut items = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // A bare JSON string is the convenient form for a quick list of
        // prompts; the object form is what you want once ids matter.
        let item = if let Ok(text) = serde_json::from_str::<String>(line) {
            BatchItem {
                id: format!("{}", i + 1),
                prompt: text.into(),
                meta: None,
            }
        } else {
            serde_json::from_str::<BatchItem>(line)
                .with_context(|| format!("{}:{}: not a valid batch item", path.display(), i + 1))?
        };
        items.push(item);
    }

    // Duplicate ids make the output impossible to join back reliably.
    let mut seen = std::collections::HashSet::new();
    for item in &items {
        anyhow::ensure!(seen.insert(&item.id), "duplicate item id {:?}", item.id);
    }

    Ok(items)
}
