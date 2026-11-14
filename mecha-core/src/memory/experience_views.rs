//! Deterministic views over original fixture evidence. Projections never rewrite
//! the source, confer authority, or consult the hidden outcome table.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trial {
    Baseline,
    RecallViews,
    Unseen,
    Checked,
}
impl Trial {
    pub fn protocol(self) -> &'static str {
        match self {
            Self::Baseline => "experience-workflow-v1",
            Self::RecallViews => "experience-recall-views-v1",
            Self::Unseen => "experience-unseen-v1",
            Self::Checked => "experience-checked-v1",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecallView {
    Legacy,
    Measurements,
    Scoped,
}
#[derive(Clone, Serialize)]
pub struct Projection {
    pub view: RecallView,
    pub text: String,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextSource>,
}
#[derive(Clone, Serialize)]
pub struct ContextSource {
    pub source: RevisionRef,
    pub text: String,
    pub sha256: String,
}
impl World {
    pub(super) fn prepare_view(
        &self,
        db: &Memory,
        at: &Snapshot,
        training: &[Trace],
        arm: Arm,
        view: RecallView,
    ) -> Result<Prepared> {
        let mut prepared = self.prepare(db, at, training, arm)?;
        if view == RecallView::Legacy {
            return Ok(prepared);
        }
        for passage in &mut prepared.request.passages {
            let mut value: Value = serde_json::from_str(&passage.text)?;
            let outcome = value
                .get_mut("outcome")
                .and_then(Value::as_object_mut)
                .context("expected fixture observation")?;
            let feasible = outcome
                .remove("feasible")
                .context("missing historical appraisal")?;
            let context = if view == RecallView::Scoped {
                let trace = training
                    .iter()
                    .find(|t| t.observation == passage.source)
                    .context("source is not a training observation")?;
                ensure!(
                    closure(&db.conn, &passage.source)?.contains(&trace.event),
                    "original context is not an observation dependency"
                );
                ensure!(
                    fresh(&db.conn, &trace.event, i64::MAX)?
                        && authorized(&db.conn, &trace.event, &access())?,
                    "context no longer current or authorized"
                );
                let Some(Record::Evidence(event)) = db.read(&trace.event, &access())? else {
                    anyhow::bail!("original event unavailable");
                };
                let event_value: Value = serde_json::from_str(&event.text)?;
                let state = event_value
                    .get("state")
                    .context("event has no original state")?;
                value["historical_appraisal"] = json!({"feasible":feasible,"original_state":state});
                Some(ContextSource {
                    source: trace.event.clone(),
                    sha256: sha(&event.text),
                    text: event.text,
                })
            } else {
                None
            };
            let text = serde_json::to_string(&value)?;
            passage.projection = Some(Projection {
                view,
                sha256: sha(&text),
                text,
                context,
            });
        }
        // The cap includes original evidence and every additional source receipt.
        ensure!(
            serde_json::to_vec(&prepared.request.passages)?.len() <= MAX_BYTES,
            "projected evidence exceeds byte budget"
        );
        prepared.request = request(&self.state(), prepared.request.passages)?;
        Ok(prepared)
    }
}

/// New names/values fixed before model evaluation. Vireo lacks Quill evidence;
/// Willow's Nori measurement is corrected from 1 to 7 minutes before recall.
/// This exercises incomplete and superseded evidence, not autonomous learning.
pub(super) fn train_unseen(db: &mut Memory) -> Result<Vec<Trace>> {
    let mut traces = Vec::new();
    for workload in 4..8 {
        let world = World::new(workload, 0);
        for method in METHODS {
            if workload == 6 && method == "Quill" {
                continue;
            }
            let mut outcome = world.outcome(method)?;
            let corrected = workload == 7 && method == "Nori";
            if corrected {
                outcome.observed_minutes = Some(1);
            }
            let mut trace = record_trace(
                db,
                &world.state(),
                json!({"kind":"host_exploration","choice":method}),
                outcome,
                &[],
                true,
            )?;
            if corrected {
                let Some(Record::Evidence(mut capture)) = db.read(&trace.observation, &access())?
                else {
                    anyhow::bail!("missing correction source");
                };
                trace.outcome = world.outcome(method)?;
                trace.observation_text =
                    json!({"workload":trace.workload,"outcome":trace.outcome}).to_string();
                capture.text.clone_from(&trace.observation_text);
                let tx = db
                    .conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                let updated = capture_in(&tx, &db.instance, capture)?;
                for input in [&trace.event, &trace.decision] {
                    ensure!(
                        fresh(&tx, input, i64::MAX)? && authorized(&tx, input, &access())?,
                        "stale correction context"
                    );
                    tx.execute(
                        "INSERT INTO dependency(output,input) VALUES (?1,?2)",
                        params![updated.revision, input.revision],
                    )?;
                }
                tx.commit()?;
                trace.previous_observation = Some(trace.observation);
                trace.observation = updated;
            }
            traces.push(trace);
        }
    }
    Ok(traces)
}
