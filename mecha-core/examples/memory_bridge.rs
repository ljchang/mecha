//! Explicit synthetic-only connected memory bridge demo; no network or model.
#[cfg(feature = "memory-prototype")]
#[path = "support/memory_bridge.rs"]
#[allow(dead_code)]
mod support;

#[cfg(feature = "memory-prototype")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2,
        "usage: memory_bridge --synthetic-only NEW_DIRECTORY"
    );
    let root = std::path::PathBuf::from(&args[1]);
    if args[0] == "--child-root" {
        return support::subprocess_goalmemory_changed_criterion(&root).await;
    }
    anyhow::ensure!(
        args[0] == "--synthetic-only",
        "explicit --synthetic-only acknowledgement required"
    );
    anyhow::ensure!(
        root.is_absolute(),
        "explicit absolute synthetic directory required"
    );
    support::connected_demo(&root, true).await?;
    println!(
        "Connected authored-provider qualification passed; artifacts: {}",
        root.display()
    );
    println!("Both exact artifacts checked independently. This does not measure memory benefit.");
    Ok(())
}

#[cfg(not(feature = "memory-prototype"))]
fn main() {
    eprintln!("This synthetic demo requires --features memory-prototype");
    std::process::exit(2);
}
