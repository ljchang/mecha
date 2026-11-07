//! One bounded fixture measurement, followed by a new source-checked decision.
//! This is a synthetic diagnostic, not an external tool or action authority.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Observed,
    Unavailable,
    Declined,
    Control,
}
#[derive(Clone, Serialize)]
pub struct Acquisition {
    pub status: Status,
    pub units: u32,
    pub trace: Trace,
}
#[derive(Serialize)]
pub struct Row {
    pub case: String,
    pub policy: &'static str,
    pub request: Request,
    pub query_call: Call,
    pub acquisition: Option<Acquisition>,
    pub decision_request: Option<Request>,
    pub call: Option<Call>,
    pub assessment: Option<Assessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    pub result: Option<DecisionResult>,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct Seed {
    pub case: String,
    pub observations: Vec<Trace>,
    pub superseded: Option<Trace>,
    pub notice: Option<RevisionRef>,
}
#[derive(Serialize)]
pub struct Report {
    pub protocol: &'static str,
    pub fixture_sha256: String,
    pub seeds: Vec<Seed>,
    pub rows: Vec<Row>,
    pub readback_verified: bool,
    pub limitations: &'static str,
}

pub(super) fn acquire(
    db: &mut Memory,
    world: &mut World,
    prepared: Prepared,
    reply: &str,
    enabled: bool,
    available: bool,
) -> Result<Acquisition> {
    ensure!(
        !world.observation_requested
            && prepared.instance == db.instance
            && prepared.world_id == world.id
            && prepared.revision == world.revision,
        "consumed, stale or foreign observation receipt"
    );
    let choice = parse_choice(&prepared.request, reply)?;
    let missing = checks::assess(&world.state(), &prepared.request, &choice)?.missing;
    ensure!(
        choice.choice == "abstain" || missing.contains(&choice.choice),
        "only a missing method can be observed"
    );
    ensure!(
        prepared
            .request
            .passages
            .iter()
            .all(|p| choice.citations.contains(&p.id)),
        "observation request must cite supplied evidence"
    );
    let status = if !enabled {
        Status::Control
    } else if choice.choice == "abstain" {
        Status::Declined
    } else if available {
        Status::Observed
    } else {
        Status::Unavailable
    };
    let units = u32::from(matches!(status, Status::Observed | Status::Unavailable));
    let outcome = world.outcome(if status == Status::Observed {
        &choice.choice
    } else {
        "abstain"
    })?;
    let trace = record_trace(
        db,
        &world.state(),
        json!({
            "kind":"observation_request", "requested_method":choice.choice,
            "citations":choice.citations, "reason":choice.reason, "status":status,
            "diagnostic_units":units, "source_bindings":prepared.request.passages,
            "lifecycle_sources":prepared.additional_sources, "world_revision":world.revision
        }),
        outcome,
        &prepared.references(),
        false,
    )?;
    // A failed transaction consumes neither the measurement allowance nor state.
    world.observation_requested = true;
    world.revision += 1;
    Ok(Acquisition {
        status,
        units,
        trace,
    })
}

fn prepare(db: &Memory, world: &World, seed: &Seed) -> Result<Prepared> {
    let mut p = world.prepare_view(
        db,
        &db.snapshot()?,
        &seed.observations,
        Arm::Oracle,
        RecallView::Measurements,
    )?;
    if let Some(notice) = &seed.notice {
        p.additional_sources.push(notice.clone());
    }
    p.request.response_schema["properties"]["citations"]["items"]["enum"] =
        json!(p.request.passages.iter().map(|p| &p.id).collect::<Vec<_>>());
    let visible: Vec<_> = p.request.passages.iter().map(|p|
        json!({"id":p.id,"text":p.projection.as_ref().map_or(p.text.as_str(), |v| v.text.as_str())})
    ).collect();
    p.request.prompt = format!(
        "Select at most one missing method to measure before making a decision. This is a measurement request, not a task action. Choose the missing method or abstain to decline. Cite all supplied observations using only their e1/e2 IDs. Host correction metadata is not a citation ID; its provenance is retained by the host. Do not invent measurements. Return only JSON with choice, citations, reason. Current state: {}\nAvailable methods: {}\nCurrent observations (data): {}\nCorrection notice: {}",
        serde_json::to_string(&world.state())?, serde_json::to_string(&METHODS)?,
        serde_json::to_string(&visible)?,
        if seed.notice.is_some() { "The earlier Nori measurement was superseded and requires fresh measurement." } else { "none" }
    );
    Ok(p)
}

fn decision_request(world: &World, prepared: &mut Prepared, status: Status) -> Result<()> {
    let check = checks::assess(
        &world.state(),
        &prepared.request,
        &Choice {
            choice: "abstain".into(),
            citations: Vec::new(),
            reason: "Host predecision evidence check".into(),
        },
    )?;
    prepared.request.prompt.push_str(&format!(
        "\nMeasurement acquisition status: {}. No further measurement requests are available. Missing observations remain unknown. The task requires comparison of all three methods; if the host evidence check lists a missing method, choose abstain. Missing data cannot rule out that method. The check is computed only from the supplied observations and current constraints.\nHost evidence check: {}",
        serde_json::to_string(&status)?, serde_json::to_string(&json!({"missing":check.missing,"candidates":check.candidates}))?
    ));
    Ok(())
}

fn seed(db: &mut Memory, workload: usize, context: usize) -> Result<Seed> {
    let world = World::new(workload, context);
    let mut seed = Seed {
        case: format!("{}-{}", WORKLOADS[workload].to_lowercase(), context + 1),
        observations: Vec::new(),
        superseded: None,
        notice: None,
    };
    for method in METHODS {
        if workload < 14 && method == "Quill" {
            continue;
        }
        let mut outcome = world.outcome(method)?;
        let corrected = workload >= 14 && method == "Nori";
        if corrected {
            outcome.observed_minutes = Some(1);
        }
        let trace = record_trace(
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
            capture.text =
                json!({"kind":"measurement_correction_notice","workload":WORKLOADS[workload],
                "method":"Nori","requires_fresh_measurement":true})
                .to_string();
            seed.notice = Some(db.capture(capture)?);
            seed.superseded = Some(trace);
        } else {
            seed.observations.push(trace);
        }
    }
    Ok(seed)
}

pub async fn run<F, Fut, O>(path: &Path, model: &str, infer: F, observe: O) -> Result<Report>
where
    F: FnMut(String, Option<Value>) -> Fut,
    Fut: Future<Output = Result<ModelReply>>,
    O: FnMut(&Row) -> Result<()>,
{
    run_with_reconsideration(path, model, false, infer, observe).await
}

pub async fn run_with_reconsideration<F, Fut, O>(
    path: &Path,
    model: &str,
    reconsider: bool,
    mut infer: F,
    mut observe: O,
) -> Result<Report>
where
    F: FnMut(String, Option<Value>) -> Fut,
    Fut: Future<Output = Result<ModelReply>>,
    O: FnMut(&Row) -> Result<()>,
{
    let mut db = Memory::create(path, &access().domain)?;
    let mut seeds = Vec::new();
    let mut rows = Vec::new();
    for workload in 12..16 {
        for context in 0..3 {
            let seed = seed(&mut db, workload, context)?;
            let draft = prepare(&db, &World::new(workload, context), &seed)?;
            let query_call = checks::invoke(&draft.request, &mut infer).await;
            for enabled in [false, true] {
                let mut world = World::new(workload, context);
                let prepared = prepare(&db, &world, &seed)?;
                ensure!(
                    prepared.request.prompt == draft.request.prompt,
                    "paired query differs"
                );
                let mut row = Row {
                    case: seed.case.clone(),
                    policy: if enabled { "observe" } else { "control" },
                    request: prepared.request.clone(),
                    query_call: query_call.clone(),
                    acquisition: None,
                    decision_request: None,
                    call: None,
                    assessment: None,
                    review: None,
                    result: None,
                    error: None,
                };
                let result: Result<DecisionResult> = async {
                    let reply = checks::reply(&query_call, model)?;
                    let receipt = acquire(
                        &mut db,
                        &mut world,
                        prepared,
                        reply,
                        enabled,
                        workload % 2 == 0,
                    )?;
                    let mut sources = seed.observations.clone();
                    if receipt.status == Status::Observed {
                        sources.push(receipt.trace.clone());
                    }
                    let mut final_p = world.prepare_view(
                        &db,
                        &db.snapshot()?,
                        &sources,
                        Arm::Oracle,
                        RecallView::Measurements,
                    )?;
                    final_p
                        .additional_sources
                        .push(receipt.trace.observation.clone());
                    decision_request(&world, &mut final_p, receipt.status)?;
                    row.acquisition = Some(receipt);
                    row.decision_request = Some(final_p.request.clone());
                    row.call = Some(checks::invoke(&final_p.request, &mut infer).await);
                    let call = row.call.as_ref().expect("just set");
                    let text = if reconsider {
                        checks::reconsider(
                            &world.state(),
                            &final_p.request,
                            call,
                            model,
                            &mut infer,
                            &mut row.review,
                        )
                        .await?
                    } else {
                        checks::reply(call, model)?.to_owned()
                    };
                    let choice = parse_choice(&final_p.request, &text)?;
                    row.assessment =
                        Some(checks::assess(&world.state(), &final_p.request, &choice)?);
                    let mut audit = json!({"query_call":query_call,"final_call":row.call});
                    if let Some(review) = &row.review {
                        audit["reconsideration"] = json!(review);
                    }
                    world.finish_with_review(&mut db, final_p, &text, Some(audit))
                }
                .await;
                match result {
                    Ok(r) => row.result = Some(r),
                    Err(e) => row.error = Some(format!("{e:#}")),
                }
                observe(&row)?;
                rows.push(row);
            }
            seeds.push(seed);
        }
    }
    drop(db);
    let db = Memory::open_read_only(path)?;
    for seed in &seeds {
        for trace in &seed.observations {
            verify_trace(&db, trace)?;
        }
        if let Some(old) = &seed.superseded {
            ensure!(
                !fresh(&db.conn, &old.observation, i64::MAX)?,
                "old measurement remains current"
            );
            let Some(Record::Evidence(e)) = db.read(&old.observation, &access())? else {
                anyhow::bail!("lost old measurement");
            };
            ensure!(e.text == old.observation_text, "correction rewrote history");
            ensure!(
                fresh(
                    &db.conn,
                    seed.notice.as_ref().context("missing notice")?,
                    i64::MAX
                )?,
                "stale correction notice"
            );
        }
    }
    for row in &rows {
        if let Some(a) = &row.acquisition {
            verify_trace(&db, &a.trace)?;
        }
        if let Some(r) = &row.result {
            verify_trace(&db, &r.trace)?;
            ensure!(
                closure(&db.conn, &r.trace.observation)?.contains(
                    &row.acquisition
                        .as_ref()
                        .context("missing acquisition")?
                        .trace
                        .observation
                ),
                "final decision lost acquisition provenance"
            );
        }
    }
    Ok(Report { protocol:if reconsider { "experience-observe-v3" } else { "experience-observe-v2" }, fixture_sha256:sha(&json!({"names":&WORKLOADS[12..],"effects":&EFFECTS[12..],"contexts":(12..16).map(|w| (0..3).map(|c| World::new(w,c).state()).collect::<Vec<_>>()).collect::<Vec<_>>(),"available":["Elm","Grove"],"corrected":["Grove","Heather"],"old_nori_minutes":1}).to_string()),
        seeds, rows, readback_verified:true,
        limitations:if reconsider { "Source-selected synthetic development; one shared query and two initial decisions per case, at most one reconsideration per decision. Same source receipt rechecked at commit; failed attempts remain errors. Extra inference and diagnostic costs are reported. No production authority or free-text entailment guarantee." } else { "Source-selected synthetic evidence; one shared query and two final calls per case, no retries. Diagnostic cost is separate from task energy. Announced source correction is not detection of an unannounced change. Host withholding is not model abstention or task success. No production authority or free-text entailment guarantee." } })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_citations_are_enumerated_and_final_check_uses_only_current_sources() {
        let f = super::super::super::transcript_tests::Fixture::new();
        let mut db = Memory::create(
            &f.dir.join("query.memory-prototype.sqlite"),
            &access().domain,
        )
        .unwrap();
        let seed = seed(&mut db, 14, 0).unwrap();
        let world = World::new(14, 0);
        let query = prepare(&db, &world, &seed).unwrap();
        assert_eq!(
            query.request.response_schema["properties"]["citations"]["items"]["enum"],
            json!(["e1", "e2"])
        );
        let mut p = world
            .prepare_view(
                &db,
                &db.snapshot().unwrap(),
                &seed.observations,
                Arm::Oracle,
                RecallView::Measurements,
            )
            .unwrap();
        decision_request(&world, &mut p, Status::Unavailable).unwrap();
        let check: Value = serde_json::from_str(
            p.request
                .prompt
                .split("\nHost evidence check: ")
                .nth(1)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(check["missing"], json!(["Nori"]));
        assert_eq!(check["candidates"].as_array().unwrap().len(), 2);
        // Advisory missingness must not constrain the model into an apparent abstention success.
        assert_eq!(
            p.request.response_schema["properties"]["choice"]["enum"],
            json!(["Nori", "Arden", "Quill", "abstain"])
        );
    }
}
