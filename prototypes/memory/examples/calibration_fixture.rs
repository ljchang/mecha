//! Deterministic, synthetic-only fixture writer; no inference or live imports.
use anyhow::{ensure, Result};
use mecha_memory_prototype::memory::*;
use serde::Deserialize;
use serde_json::json;
use std::{io::Read, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    evidence: String,
    concept: bool,
}
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("NEW.memory-prototype.sqlite required");
    let mut input = String::new();
    std::io::stdin().take(20_001).read_to_string(&mut input)?;
    ensure!(input.len() <= 20_000, "fixture too large");
    let input: Input = serde_json::from_str(&input)?;
    let mut db = Memory::create(Path::new(&path), "synthetic-demo")?;
    let capture = |db: &mut Memory, id: &str, text: &str| -> Result<RevisionRef> {
        db.capture(Capture {
            source: SourceKey {
                connector: "fixture".into(),
                account: "synthetic-calibration".into(),
                id: id.into(),
            },
            text: text.into(),
            occurred_at: None,
            scopes: fixture::access().scopes,
            transcript: None,
        })
    };
    let evidence = capture(&mut db, "case-evidence", &input.evidence)?;
    let concept = if input.concept {
        let texts = [
            "A bridge closure blocked Parcel J. A permitted ferry route delivered it.",
            "A road closure blocked Parcel K. An approved air route delivered it.",
            "All carriers moving Parcel L needed a certificate. Switching carriers did not remove the missing certificate requirement.",
        ];
        let mut refs = Vec::new();
        for (i, text) in texts.iter().enumerate() {
            refs.push(capture(&mut db, &format!("analogy-{i}"), text)?);
        }
        let citation = |i: usize| json!({"source_id":refs[i].revision,"field":"","quote":texts[i]});
        let draft = json!({"name":"Alternative route", "principle":"An available alternate route can overcome an obstacle specific to the original route.",
            "applicability":"The alternative must be permitted; shared requirements remain binding.",
            "support":[citation(0),citation(1)],"counterexamples":[citation(2)]});
        let receipt = db.prepare_concept(&refs, &db.snapshot()?, &fixture::access())?;
        Some(db.retain_concept(receipt, &draft.to_string(), &fixture::access(), None)?)
    } else {
        None
    };
    println!(
        "{}",
        json!({"evidence":evidence,"concept":concept,"fixture_only":true})
    );
    Ok(())
}
