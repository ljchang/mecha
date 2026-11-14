//! No-model replay of frozen search plans with explicit human mention bindings.
//! Diagnostic on already-seen cases; never a new held-out reasoning score.
use super::*;
use serde::Deserialize;

const PLANS: &str = include_str!("fixtures/mention_replay.json");
#[derive(Deserialize)]
struct Replay {
    source_report_sha256: String,
    source_protocol: String,
    fixture_sha256: String,
    annotation: String,
    plans: Vec<Plan>,
}
#[derive(Deserialize)]
struct Plan {
    case_id: String,
    queries: Vec<Search>,
}
#[derive(Deserialize, Serialize)]
struct Search {
    text: String,
    mention: Option<String>,
}
fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn words(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}
fn contains_phrase(text: &str, phrase: &str) -> bool {
    format!(" {} ", words(text)).contains(&format!(" {} ", words(phrase)))
}
fn validate_plan(plan: &Plan, initial: &[Evidence]) -> Result<()> {
    parse_queries(&serde_json::to_string(
        &json!({"queries":plan.queries.iter().map(|q|&q.text).collect::<Vec<_>>()}),
    )?)?;
    for q in &plan.queries {
        if let Some(name) = &q.mention {
            ensure!(
                !words(name).is_empty() && name.len() <= 256,
                "invalid annotated mention"
            );
            ensure!(
                contains_phrase(&q.text, name),
                "mention absent from saved query"
            );
            // Require original visible message text, not UUIDs, source IDs or metadata.
            ensure!(
                initial.iter().any(|e| match &e.hit.record {
                    Record::Evidence(x) => x
                        .transcript
                        .as_ref()
                        .is_some_and(|t| t.fields.iter().any(|f| contains_phrase(&f.text, name))),
                    _ => false,
                }),
                "mention absent from initial evidence"
            );
        }
    }
    Ok(())
}
fn ids(evidence: &[Evidence]) -> Vec<&str> {
    evidence.iter().map(|e| e.id.as_str()).collect()
}
fn anchor_ids(hits: &[RecallHit]) -> Result<Vec<String>> {
    hits.iter()
        .map(|h| evidence(h.anchor.clone()).map(|e| e.id))
        .collect()
}

/// Create a new synthetic database and compare identical frozen searches.
/// Gold enters coverage only; manually bound names are disclosed as extra input.
pub fn run(path: &Path) -> Result<serde_json::Value> {
    let replay: Replay = serde_json::from_str(PLANS)?;
    ensure!(
        replay.fixture_sha256 == hash(HOLDOUT),
        "replay fixture hash mismatch"
    );
    let d = load(Split::Holdout)?;
    let plan_ids: BTreeSet<_> = replay.plans.iter().map(|p| p.case_id.as_str()).collect();
    ensure!(
        replay.plans.len() == d.cases.len()
            && plan_ids.len() == d.cases.len()
            && d.cases.iter().all(|c| plan_ids.contains(c.id.as_str())),
        "replay must cover every case exactly once"
    );
    let (memory, _) = prepare(path, &d)?;
    let snapshot = memory.snapshot()?;
    let mut rows = Vec::new();
    for c in &d.cases {
        let plan = replay
            .plans
            .iter()
            .find(|p| p.case_id == c.id)
            .context("missing saved plan")?;
        for bound in [false, true] {
            let started = Instant::now();
            let mut searches = vec![search(&memory, c, &c.question, &snapshot)?];
            let initial = fuse(&searches)?;
            validate_plan(plan, &initial)?;
            let mut traces = vec![
                json!({"text":c.question,"required_mention":null,"anchor_ids":anchor_ids(&searches[0])?}),
            ];
            for q in &plan.queries {
                let mention = q.mention.as_deref().filter(|_| bound);
                let hits = match mention {
                    Some(name) => memory.recall_mention(
                        &search_query(c, &q.text, &snapshot),
                        name,
                        &fixture::access(),
                    )?,
                    None => search(&memory, c, &q.text, &snapshot)?,
                };
                traces.push(json!({"text":q.text,"required_mention":mention,"anchor_ids":anchor_ids(&hits)?}));
                searches.push(hits);
            }
            let evidence = fuse(&searches)?;
            let evidence_ids = ids(&evidence);
            let gold = gold_ids(c);
            let coverage = (!gold.is_empty()).then(|| {
                gold.iter()
                    .filter(|id| evidence_ids.contains(&id.as_str()))
                    .count() as f64
                    / gold.len() as f64
            });
            rows.push(json!({"case_id":c.id,"category":c.category,"arm":if bound {"required_mention"} else {"original"},
                "annotations":plan.queries,"initial_ids":ids(&initial),"searches":traces,"search_count":searches.len(),
                "evidence_ids":evidence_ids,"evidence_bytes":serde_json::to_vec(&evidence)?.len(),
                "coverage":coverage,"retrieval_micros":started.elapsed().as_micros()}));
        }
    }
    Ok(
        json!({"protocol":"mention-replay-v1","fixture_only":true,"dataset_use":"development_replay",
        "fixture_sha256":hash(HOLDOUT),"plans_sha256":hash(PLANS),"source_report_sha256":replay.source_report_sha256,
        "source_protocol":replay.source_protocol,"annotation":replay.annotation,
        "model_calls":0,"evidence_budget":EVIDENCE_BUDGET,"max_evidence":MAX_EVIDENCE,"rows":rows,
        "limitations":["manually annotated names are extra input, not automatic entity extraction or resolution",
        "already-seen synthetic cases; no held-out or model-answer claim",
        "same search count and evidence cap, but mention filtering builds an additional eligible-only index",
        "neighbors may omit the required mention; only anchors are constrained",
        "lexical phrases cannot resolve aliases, coreference or homonyms; no production-scale qualification"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn annotation_cannot_name_an_unseen_entity_or_one_missing_from_query() {
        let f = super::super::super::super::transcript_tests::Fixture::new();
        let d = load(Split::Holdout).unwrap();
        let (memory, _) = prepare(&f.dir.join("annotations.memory-prototype.sqlite"), &d).unwrap();
        let c = &d.cases[0];
        let initial =
            fuse(&[search(&memory, c, &c.question, &memory.snapshot().unwrap()).unwrap()]).unwrap();
        for (text, name) in [
            ("Invented capacity", "Invented"),
            ("room capacity", "Vireo"),
        ] {
            assert!(validate_plan(
                &Plan {
                    case_id: c.id.clone(),
                    queries: vec![Search {
                        text: text.into(),
                        mention: Some(name.into())
                    }]
                },
                &initial
            )
            .is_err());
        }
    }
}
