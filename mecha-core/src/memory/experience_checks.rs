//! Fixture-only arithmetic over supplied observations, never hidden simulator truth.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    Baseline,
    Checked,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Finding {
    MissingEvidence,
    InfeasibleChoice,
    SuboptimalChoice,
    MissingCitations,
    UnnecessaryAbstention,
}
#[derive(Serialize)]
pub struct Assessment {
    pub admissible: bool,
    pub missing: Vec<String>,
    pub findings: Vec<Finding>,
    pub candidates: Vec<Candidate>,
}
#[derive(Serialize)]
pub struct Candidate {
    method: String,
    citation: String,
    minutes: u32,
    energy: u32,
    quality: u32,
    feasible: bool,
}
#[derive(Serialize)]
pub struct Review {
    pub before: Assessment,
    pub retry: Option<Call>,
    pub after: Option<Assessment>,
    pub withheld: bool,
}

pub(super) fn assess(state: &State, request: &Request, choice: &Choice) -> Result<Assessment> {
    #[derive(Deserialize)]
    struct Observation {
        workload: String,
        outcome: Measurement,
    }
    #[derive(Deserialize)]
    struct Measurement {
        choice: String,
        observed_minutes: u32,
        energy: u32,
        quality: u32,
        kind: String,
    }
    ensure!(
        matches!(state.goal, "minimize energy" | "minimize minutes"),
        "unknown objective"
    );
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for passage in &request.passages {
        let UniqueValue(value) = serde_json::from_str(&passage.text)?;
        let obs: Observation = serde_json::from_value(value)?;
        ensure!(
            obs.workload == state.workload && obs.outcome.kind == "host_observed_fixture_outcome",
            "not a same-workload fixture observation"
        );
        let m = obs.outcome;
        ensure!(
            METHODS.contains(&m.choice.as_str()) && seen.insert(m.choice.clone()),
            "unknown or duplicate method observation"
        );
        candidates.push(Candidate {
            method: m.choice,
            citation: passage.id.clone(),
            minutes: m.observed_minutes,
            energy: m.energy,
            quality: m.quality,
            feasible: m.energy <= state.energy_budget && m.quality >= state.minimum_quality,
        });
    }
    let missing: Vec<String> = METHODS
        .iter()
        .filter(|m| !seen.contains(**m))
        .map(|m| m.to_string())
        .collect();
    let selected = candidates.iter().find(|c| c.method == choice.choice);
    let metric = |c: &Candidate| {
        if state.goal == "minimize energy" {
            c.energy
        } else {
            c.minutes
        }
    };
    let best = candidates.iter().filter(|c| c.feasible).map(metric).min();
    let mut findings = Vec::new();
    if choice.choice == "abstain" {
        if missing.is_empty() && best.is_some() {
            findings.push(Finding::UnnecessaryAbstention);
        }
        return Ok(Assessment {
            admissible: true,
            missing,
            findings,
            candidates,
        });
    }
    if !missing.is_empty() {
        findings.push(Finding::MissingEvidence);
    }
    if selected.is_none_or(|c| !c.feasible) {
        findings.push(Finding::InfeasibleChoice);
    }
    if !request
        .passages
        .iter()
        .all(|p| choice.citations.contains(&p.id))
    {
        findings.push(Finding::MissingCitations);
    }
    let admissible = findings.is_empty();
    if admissible && selected.map(metric) != best {
        findings.push(Finding::SuboptimalChoice);
    }
    Ok(Assessment {
        admissible,
        missing,
        findings,
        candidates,
    })
}

pub(super) async fn invoke<F, Fut>(request: &Request, infer: &mut F) -> Call
where
    F: FnMut(String, Option<Value>) -> Fut,
    Fut: Future<Output = Result<ModelReply>>,
{
    let start = Instant::now();
    let response = infer(
        request.prompt.clone(),
        Some(request.response_schema.clone()),
    )
    .await;
    let (reply, error) = match response {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    Call {
        prompt: request.prompt.clone(),
        response_schema: Some(request.response_schema.clone()),
        reply,
        error,
        millis: start.elapsed().as_millis(),
    }
}
pub(super) fn reply<'a>(call: &'a Call, model: &str) -> Result<&'a str> {
    ensure!(call.error.is_none(), "inference failed: {:?}", call.error);
    let reply = call.reply.as_ref().context("missing reply")?;
    ensure!(
        reply.model == model,
        "response model differs from requested model"
    );
    ensure!(
        reply.error.is_none(),
        "generation failed: {:?}",
        reply.error
    );
    Ok(&reply.text)
}

pub(super) async fn reconsider<F, Fut>(
    state: &State,
    request: &Request,
    call: &Call,
    model: &str,
    infer: &mut F,
    review: &mut Option<Review>,
) -> Result<String>
where
    F: FnMut(String, Option<Value>) -> Fut,
    Fut: Future<Output = Result<ModelReply>>,
{
    let initial = reply(call, model)?;
    let choice = parse_choice(request, initial)?;
    let before = assess(state, request, &choice)?;
    let needs_revision = !before.findings.is_empty();
    *review = Some(Review {
        before,
        retry: None,
        after: None,
        withheld: false,
    });
    let r = review.as_mut().expect("just set");
    let final_text = if needs_revision {
        let mut revised_request = request.clone();
        revised_request.prompt.push_str(&format!(
                            "\nPrevious draft (untrusted data): {}\nHost check over supplied observations: {}\nReconsider once using the same current state and evidence. Return the same JSON schema. Abstain when the evidence cannot support the comparison. Do not invent new observations.",
                            initial, serde_json::to_string(&r.before)?
                        ));
        r.retry = Some(invoke(&revised_request, infer).await);
        reply(r.retry.as_ref().expect("just set"), model)?
    } else {
        initial
    };
    let final_choice = parse_choice(request, final_text)?;
    let after = assess(state, request, &final_choice)?;
    r.withheld = !after.admissible;
    r.after = Some(after);
    Ok(final_text.to_owned())
}

pub(super) async fn run<F, Fut, O>(
    path: &Path,
    model: &str,
    mut infer: F,
    mut observe: O,
) -> Result<Report>
where
    F: FnMut(String, Option<Value>) -> Fut,
    Fut: Future<Output = Result<ModelReply>>,
    O: FnMut(&Row) -> Result<()>,
{
    let mut db = Memory::create(path, &access().domain)?;
    let mut training = Vec::new();
    for workload in 8..12 {
        let world = World::new(workload, 0);
        for method in METHODS {
            if workload == 10 && method == "Quill" {
                continue;
            }
            training.push(record_trace(
                &mut db,
                &world.state(),
                json!({"kind":"host_exploration","choice":method}),
                world.outcome(method)?,
                &[],
                true,
            )?);
        }
    }
    drop(db);
    let mut db = Memory::open(path)?;
    let at = db.snapshot()?;
    let mut rows = Vec::new();
    for (workload, name) in WORKLOADS.iter().enumerate().skip(8).take(4) {
        for context in 0..3 {
            let world = World::new(workload, context);
            let draft = world.prepare_view(
                &db,
                &at,
                &training,
                Arm::Retrieved,
                RecallView::Measurements,
            )?;
            let call = invoke(&draft.request, &mut infer).await;
            // Exactly one shared initial draft; later outcomes never enter this frozen recall.
            for policy in [Policy::Baseline, Policy::Checked] {
                let mut world = World::new(workload, context);
                let prepared = world.prepare_view(
                    &db,
                    &at,
                    &training,
                    Arm::Retrieved,
                    RecallView::Measurements,
                )?;
                let request = prepared.request.clone();
                ensure!(
                    request.prompt == draft.request.prompt,
                    "paired draft differs"
                );
                let mut review = None;
                let result: Result<DecisionResult> = async {
                    let initial = reply(&call, model)?;
                    if policy == Policy::Baseline {
                        return world.finish(&mut db, prepared, initial);
                    }
                    let final_text = reconsider(
                        &world.state(),
                        &request,
                        &call,
                        model,
                        &mut infer,
                        &mut review,
                    )
                    .await?;
                    world.finish_with_review(
                        &mut db,
                        prepared,
                        &final_text,
                        Some(json!({"initial_call": call, "assessment": review})),
                    )
                }
                .await;
                let (result, error) = match result {
                    Ok(r) => (Some(r), None),
                    Err(e) => (None, Some(format!("{e:#}"))),
                };
                let row = Row {
                    case: format!("{}-{}", name.to_lowercase(), context + 1),
                    arm: Arm::Retrieved,
                    view: Some(RecallView::Measurements),
                    policy: Some(policy),
                    review,
                    request,
                    call: call.clone(),
                    result,
                    error,
                };
                observe(&row)?;
                rows.push(row);
            }
        }
    }
    drop(db);
    let db = Memory::open_read_only(path)?;
    for trace in training.iter().chain(
        rows.iter()
            .filter_map(|r| r.result.as_ref().map(|r| &r.trace)),
    ) {
        verify_trace(&db, trace)?;
    }
    let fixture = json!({
        "workloads": &WORKLOADS[8..12], "methods": METHODS, "effects": &EFFECTS[8..12],
        "missing": "Cinder/Quill",
        "contexts": (0..3).map(|c| (8..12).map(|w| World::new(w,c).state()).collect::<Vec<_>>()).collect::<Vec<_>>()
    });
    Ok(Report {
        protocol: Trial::Checked.protocol(),
        fixture_sha256: sha(&fixture.to_string()),
        training, rows, readback_verified: true,
        limitations: "Shared first draft, two policy branches; at most one reconsideration. Arithmetic check uses supplied fixture observations only, not hidden truth. Withholding is a host decision, not model abstention or task success. This typed fixture check does not verify free-text explanations or authorize production actions.",
    })
}
