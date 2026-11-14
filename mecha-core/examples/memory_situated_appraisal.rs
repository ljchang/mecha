//! Authored mechanics demonstration; no network, model-quality claim or live data.
#[cfg(feature = "memory-prototype")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2 && args[0] == "--synthetic-only",
        "usage: memory_situated_appraisal --synthetic-only NEW_ABSOLUTE_DIRECTORY"
    );
    let report =
        mecha_core::memory_situated_workflow::run_authored(std::path::Path::new(&args[1])).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
#[cfg(not(feature = "memory-prototype"))]
fn main() {
    eprintln!("requires --features memory-prototype");
    std::process::exit(1);
}
