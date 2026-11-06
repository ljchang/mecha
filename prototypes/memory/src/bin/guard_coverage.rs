//! Synthetic qualification adapter. Reads one bounded contract/plan JSON from stdin.
//! Not an agent tool or a replacement for the Mecha CLI. No actions execute.
use std::io::Read;
fn main() -> anyhow::Result<()> {
    anyhow::ensure!(
        std::env::args().len() == 1,
        "Supply one synthetic contract/plan JSON on stdin; no arguments"
    );
    let mut input = String::new();
    std::io::stdin().take(65_537).read_to_string(&mut input)?;
    let report = mecha_memory_prototype::memory::guard_coverage::check_json(&input)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
