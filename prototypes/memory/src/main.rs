use anyhow::{bail, Result};
use mecha_memory_prototype::memory::*;
use serde_json::json;
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [command, path] if command == "demo" => fixture::demo(Path::new(path))?,
        [command, path] if command == "inspect" => {
            let memory = Memory::open(Path::new(path))?;
            json!({"snapshot":memory.snapshot()?,"fixture_only":true})
        }
        [command, path, source] if command == "forget" => {
            let mut memory = Memory::open(Path::new(path))?;
            serde_json::to_value(memory.forget(&fixture::source_key(source))?)?
        }
        [command, path, text, rest @ ..] if command == "query" && rest.len() <= 2 => {
            let memory = Memory::open(Path::new(path))?;
            let mut snapshot = memory.snapshot()?;
            if let Some(sequence) = rest.first().filter(|s| s.as_str() != "latest") {
                snapshot.sequence = sequence.parse()?;
            }
            let query = Query { text:text.clone(), known_at:snapshot, valid_at:rest.get(1).cloned(), include_possible:true, limit:20 };
            serde_json::to_value(memory.search(&query, &fixture::access())?)?
        }
        _ => bail!("Usage (synthetic fixtures only):\n  mecha-memory-prototype demo NEW.memory-prototype.sqlite\n  mecha-memory-prototype query FILE TEXT [KNOWN_SEQ|latest] [VALID_AT_RFC3339]\n  mecha-memory-prototype inspect FILE\n  mecha-memory-prototype forget FILE SOURCE_ID\nNo live-store import, connectors, or nightly jobs are enabled."),
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
