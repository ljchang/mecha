//! Deterministic synthetic fixtures shared by experimental frontends.
//! No source connectors or private-store discovery.
use super::*;
use anyhow::Result;
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path};

pub fn access() -> Access {
    Access {
        domain: "synthetic-demo".into(),
        scopes: demo_scopes(),
    }
}

fn demo_scopes() -> BTreeSet<String> {
    ["fixture-mail".into()].into_iter().collect()
}
pub fn source_key(id: &str) -> SourceKey {
    SourceKey {
        connector: "fixture".into(),
        account: "synthetic".into(),
        id: id.into(),
    }
}
pub fn demo(path: &Path) -> Result<Value> {
    let mut memory = Memory::create(path, "synthetic-demo")?;
    let text = "Person A works at Organization B, starting May 1.";
    let evidence = memory.capture(Capture {
        source: source_key("employment"),
        text: text.into(),
        occurred_at: Some("2026-06-01T09:00:00-04:00".into()),
        scopes: demo_scopes(),
        transcript: None,
    })?;
    let person = memory.entity("Person A", BTreeSet::new())?;
    let org = memory.entity("Organization B", BTreeSet::new())?;
    let mut relation = RelationInput {
        predicate: "works_at".into(),
        subject: person,
        object: Target::Entity(org),
        support: Span {
            evidence: evidence.clone(),
            start: 0,
            end: text.len(),
        },
        consulted: vec![],
        text: "Person A works at Organization B".into(),
        polarity: 1,
        valid: Interval {
            from: Bound::Known("2026-05-01T00:00:00Z".into()),
            to: Bound::Unbounded,
        },
    };
    let initial = memory.relate(relation.clone())?;
    let before = memory.snapshot()?;
    let departure = "Person A left Organization B on June 30.";
    let update = memory.capture(Capture {
        source: source_key("departure"),
        text: departure.into(),
        occurred_at: Some("2026-07-10T09:00:00-04:00".into()),
        scopes: demo_scopes(),
        transcript: None,
    })?;
    relation.support = Span {
        evidence: update,
        start: 0,
        end: departure.len(),
    };
    relation.consulted = vec![evidence];
    relation.valid.to = Bound::Known("2026-06-30T00:00:00Z".into());
    memory.revise_relation(&initial, relation)?;
    Ok(
        json!({"fixture_only":true,"snapshots":{"before_departure":before,"after_departure":memory.snapshot()?},
        "source_ids":["employment","departure"],
        "example_question":"What did we know at the first snapshot about employment on July 1?",
        "limitations":["synthetic data; unencrypted SQLite","manually specified grounded relations; no model extraction","no live mecha-graph import or scheduling"]}),
    )
}
