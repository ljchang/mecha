//! `mecha corrections` — what the owner's acts have taught, as kept.
//!
//! A readout of `mecha_core::attempts`: how many turns were kept as
//! attempts, how many the owner judged and how, how many carry their words,
//! how often something was handed to a run, and the latest judgements. No
//! model and no network, and it reads whether or not `[features]
//! corrections` is on: the file is the owner's either way, and switching the
//! feature off stops it growing, not being read.

use anyhow::Result;
use mecha_core::attempts::{Source, Store};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Machine-readable.
    #[arg(long)]
    pub json: bool,

    /// How many of the latest judgements to list.
    #[arg(long, default_value_t = 10)]
    pub recent: usize,
}

pub async fn execute(args: Args) -> Result<()> {
    let store = Store::in_home(&mecha_core::work::mecha_home()?);
    let readout = store.load()?.readout(args.recent);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&readout)?);
        return Ok(());
    }
    println!("{} turn(s) kept as attempts", readout.attempts);
    println!(
        "  judged against   {}  ({} with your words or the fields you changed)",
        readout.judged_against, readout.with_a_reason
    );
    println!("  judged good      {}", readout.judged_good);
    println!(
        "  not judged       {}  — kept, and handed to nothing",
        readout.unjudged
    );
    println!("{} time(s) something was handed to a run", readout.handed);
    if readout.unreadable > 0 {
        println!(
            "{} line(s) of {} could not be read",
            readout.unreadable,
            store.path().display()
        );
    }
    if !readout.recent.is_empty() {
        println!("\nlatest judgements against an attempt");
    }
    for judged in &readout.recent {
        let how = match judged.source {
            Source::Edit => "you edited the draft",
            Source::Reject => "you rejected the draft",
            Source::Denial => "you refused a call",
            Source::Steer => "you stepped in",
            Source::SentUnchanged | Source::Unknown => "judged",
        };
        println!(
            "  {}  {}  {}",
            judged.when.get(..10).unwrap_or(&judged.when),
            how,
            judged.session
        );
        println!("    asked: {}", judged.asked);
        if let Some(why) = &judged.why {
            println!("    {why}");
        }
    }
    Ok(())
}
