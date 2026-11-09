//! Authored responses verify the workflow boundary, not semantic judgment quality.
use super::*;
use mecha_core::{
    memory::interpretation::{ConceptApplicability, InterpretationContext, NativeCriterionState},
    memory_factual_support::GenerationBudget,
    memory_situated_workflow::{appraise_native_session, NativeSessionAppraisalRequest},
};

struct Appraise {
    changed: bool,
    withdraw: Option<(PathBuf, String)>,
}

#[async_trait::async_trait]
impl Provider for Appraise {
    fn id(&self) -> &str {
        "authored-concept-appraisal"
    }
    fn default_model(&self) -> &str {
        "authored-no-model"
    }
    fn local(&self) -> bool {
        true
    }
    fn structured_output(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
        _: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        ensure!(request.tools.is_empty(), "advisory appraisal exposed tools");
        ensure!(
            request.response_schema.is_some(),
            "appraisal lost constrained output"
        );
        let text = request
            .messages
            .last()
            .context("missing appraisal input")?
            .text();
        let start = text.find('{').context("missing appraisal JSON")?;
        let end = text.rfind('}').context("missing appraisal JSON end")?;
        let view: Value = serde_json::from_str(&text[start..=end])?;
        ensure!(
            view["protocol"] == "situated-native-concept-appraisal-v1",
            "wrong appraisal protocol"
        );
        let facts = view["facts"].as_array().context("missing facts")?;
        let hypotheses = view["hypotheses"]
            .as_array()
            .context("missing hypotheses")?;
        ensure!(hypotheses.len() == 1, "expected one separate hypothesis");
        for hypothesis in hypotheses {
            ensure!(
                hypothesis["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("hypothesis:")),
                "wrong hypothesis namespace"
            );
            ensure!(
                hypothesis["concept"]["status"] == "provisional",
                "hypothesis promoted"
            );
            for kind in ["support", "counterexamples"] {
                for source in hypothesis["concept"][kind]
                    .as_array()
                    .context("missing concept citations")?
                {
                    ensure!(
                        facts
                            .iter()
                            .any(|fact| fact["source"] == source["evidence"]),
                        "concept citation lost its visible factual evidence"
                    );
                }
            }
        }
        ensure!(
            facts.iter().all(|fact| fact["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("fact:"))),
            "hypothesis entered facts"
        );
        let artifact = facts
            .iter()
            .find(|fact| {
                fact["metadata"]["role"] == "ArtifactObservation"
                    && fact["metadata"]["captures"]
                        .as_array()
                        .is_some_and(|captures| {
                            captures
                                .iter()
                                .any(|capture| capture["relation"] == "SelectedCapture")
                        })
            })
            .context("missing current observed artifact")?;
        let complete = artifact["value"]["value"]["content_status"] == "Complete";
        let response = json!({"appraisal":{
            "request_key":view["request_key"],
            "artifact_content":if complete {"complete"} else {"unavailable"},
            "artifact_facts":[artifact["id"]],
            "comparison":if complete {"supports"} else {"insufficient"},
            "because":[artifact["id"]],
            "reason":"Authored synthetic criterion judgment.",
            "questions":if complete {json!([])} else {json!(["What output was actually written?"])},
            "state":if complete {"met"} else {"unknown"}},
            "hypotheses":[{"hypothesis_id":hypotheses[0]["id"],
                "applicability":if self.changed {"inapplicable"} else {"applicable"},
                "because":if self.changed {json!([])} else {json!([artifact["id"]])},
                "reason":if self.changed {"The new criterion concerns a different property."} else {"The criterion concerns the observed output."},
                "questions":[]}]});
        if let Some((path, locator)) = &self.withdraw {
            LocalAuthority::open(path)?.withdraw(locator)?;
        }
        Ok(CompletionResponse {
            message: Message::assistant(vec![Block::text(response.to_string())]),
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            refusal: None,
            model: self.default_model().into(),
            malformed_tool_args: 0,
        })
    }
}

fn request(
    fixture: &Fixture,
    report: &TaskReport,
    changed: bool,
) -> Result<NativeSessionAppraisalRequest> {
    let mut criterion = fixture.request.criterion.clone();
    if changed {
        criterion.criterion_text = "output.json must contain a syntactically valid array.".into();
        criterion.criterion_revision = "8".repeat(64);
    }
    Ok(NativeSessionAppraisalRequest {
        receipt: report
            .receipt
            .clone()
            .context("missing recapture receipt")?,
        previous: vec![],
        context: InterpretationContext::new(
            criterion,
            if changed {
                "changed-native-context"
            } else {
                "native-context"
            }
            .into(),
        )?,
        access: fixture.access(),
        model: "authored-no-model".into(),
        budget: GenerationBudget::new(1024, std::time::Duration::from_secs(10))?,
    })
}

#[tokio::test]
async fn reopened_native_session_appraisal_keeps_hypotheses_provisional_and_sources_revocable(
) -> Result<()> {
    let fixture = Fixture::new()?;
    let access = fixture.access();
    let (first, _) = fixture
        .run("appraisal-first", Task::Independent(READY))
        .await?;
    let (second, _) = fixture
        .run("appraisal-second", Task::Independent(READY))
        .await?;
    let mut db = fixture.open()?;
    let (a, a_sources) = episode(&db, &first, &access, READY, CheckVerdict::Passed)?;
    let (_, b_sources) = episode(&db, &second, &access, READY, CheckVerdict::Passed)?;
    let support = [a_sources[0].clone(), b_sources[0].clone()];
    let refs = a_sources
        .iter()
        .chain(&b_sources)
        .map(|e| e.evidence.clone())
        .collect::<Vec<_>>();
    let prepared = db.prepare_native_concept(&refs, &db.snapshot()?, &access)?;
    let concept = db.retain_native_concept(
        prepared,
        &draft(INITIAL, &support, &[]).to_string(),
        &access,
        None,
    )?;
    drop(db);

    let (reuse, packet) = fixture.run("appraisal-reuse", Task::Reuse).await?;
    assert!(reuse.taint.private && reuse.taint.untrusted);
    assert!(references(&packet.context("missing native recall")?)?.contains(&concept));
    let db = fixture.open()?;
    let req = request(&fixture, &reuse, false)?;
    assert!(db
        .prepare_interpretation(&req.receipt, &[], &req.context, &db.snapshot()?, &access)
        .is_err());
    assert!(db
        .prepare_native_concept(
            &[req.receipt.originals[0].clone(), refs[0].clone()],
            &db.snapshot()?,
            &access
        )
        .is_err());
    let before = serde_json::to_value(db.snapshot()?)?;
    drop(db);
    let authority = LocalAuthority::open(&fixture.authority)?;
    let result = appraise_native_session(
        &fixture.database,
        &authority,
        &req,
        &Appraise {
            changed: false,
            withdraw: None,
        },
        &CancellationToken::new(),
    )
    .await?;
    let appraisal = result.result.map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert_eq!(appraisal.appraisal.state, NativeCriterionState::Met);
    assert_eq!(
        appraisal.hypotheses[0].applicability,
        ConceptApplicability::Applicable
    );
    assert!(result.raw_reply.is_some());
    assert_eq!(serde_json::to_value(fixture.open()?.snapshot()?)?, before);

    let changed = request(&fixture, &reuse, true)?;
    let result = appraise_native_session(
        &fixture.database,
        &authority,
        &changed,
        &Appraise {
            changed: true,
            withdraw: None,
        },
        &CancellationToken::new(),
    )
    .await?;
    let changed_appraisal = result.result.map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert_ne!(
        appraisal.appraisal.request_key,
        changed_appraisal.appraisal.request_key
    );
    assert_eq!(
        changed_appraisal.hypotheses[0].applicability,
        ConceptApplicability::Inapplicable
    );
    assert_eq!(changed_appraisal.appraisal.state, NativeCriterionState::Met);

    let (wrong, _) = fixture
        .run("appraisal-opposition", Task::Independent(WRONG))
        .await?;
    let mut db = fixture.open()?;
    let (_, opposition) = episode(&db, &wrong, &access, WRONG, CheckVerdict::Failed)?;
    let prepared =
        db.prepare_native_concept_rumination(&concept, &opposition[..1], &db.snapshot()?, &access)?;
    let RuminationOutcome::RevisedProvisional { reference: revised } = db
        .finish_native_concept_rumination(
        prepared,
        &json!({"outcome":"revised_provisional","draft":draft(REVISED,&support,&opposition[..1])})
            .to_string(),
        &access,
    )?
    else {
        anyhow::bail!("revision missing")
    };
    drop(db);
    let (inspection, packet) = fixture.run("appraisal-revised-read", Task::Inspect).await?;
    assert!(references(&packet.context("revised recall missing")?)?.contains(&revised));
    let req = request(&fixture, &inspection, false)?;
    let result = appraise_native_session(
        &fixture.database,
        &authority,
        &req,
        &Appraise {
            changed: false,
            withdraw: None,
        },
        &CancellationToken::new(),
    )
    .await?;
    assert_eq!(
        result
            .result
            .map_err(|e| anyhow::anyhow!("{e:?}"))?
            .appraisal
            .state,
        NativeCriterionState::Unknown
    );

    let locator = a["binding"]["locator"]
        .as_str()
        .context("missing original locator")?
        .to_string();
    let withdrawn = appraise_native_session(
        &fixture.database,
        &authority,
        &req,
        &Appraise {
            changed: false,
            withdraw: Some((fixture.authority.clone(), locator)),
        },
        &CancellationToken::new(),
    )
    .await;
    assert!(
        withdrawn.is_err(),
        "source withdrawal during inference returned an outcome"
    );
    Ok(())
}
