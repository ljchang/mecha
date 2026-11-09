//! Experiment-only source withdrawal. Never opens a database or runs a model.
#[cfg(feature = "memory-prototype")]
use anyhow::{ensure, Context, Result};
#[cfg(feature = "memory-prototype")]
use mecha_core::memory_experience_bridge::LocalAuthority;
#[cfg(feature = "memory-prototype")]
use std::path::Path;

#[cfg(feature = "memory-prototype")]
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3 && args[0] == "--synthetic-only",
        "usage: memory_native_withdraw --synthetic-only ABS_AUTHORITY CONFIRMATION"
    );
    let root = Path::new(&args[1]);
    ensure!(root.is_absolute(), "explicit absolute authority required");
    let confirmation = &args[2];
    ensure!(
        confirmation.len() == 64 && confirmation.bytes().all(|b| b.is_ascii_hexdigit()),
        "bounded confirmation digest required"
    );
    let authority = LocalAuthority::open(root).context("open synthetic authority")?;
    let binding = authority.binding(confirmation)?;
    authority.withdraw(&binding.locator)?;
    println!("{}", serde_json::to_string(&binding)?);
    Ok(())
}

#[cfg(not(feature = "memory-prototype"))]
fn main() {
    eprintln!("memory_native_withdraw requires --features memory-prototype");
    std::process::exit(2);
}
