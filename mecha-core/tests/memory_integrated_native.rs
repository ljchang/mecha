//! Authored synthetic lifecycle qualification. No model inference or quality claim.
#![cfg(feature = "memory-prototype")]

use anyhow::{ensure, Context, Result};
use mecha_core::{
    memory::{
        concept::{rumination::RuminationOutcome, Concept, ConceptExcerpt, ConceptStatus},
        host_capture::{
            ActionInvocation, ArtifactObservation, CheckVerdict, ConfirmedCriterion, ContentStatus,
        },
        Access, KeyFilePolicy, Memory, Record, RevisionRef, StorageAccess,
    },
    memory_experience_bridge::LocalAuthority,
    memory_situated_workflow::{
        run_native_session, run_native_session_with_storage, NativeSessionRequest, TaskReport,
    },
    message::{Block, CompletionRequest, CompletionResponse, Message, StopReason, Usage},
    provider::{Provider, StreamSink},
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

const NAME: &str = "Corundum";
const GOAL: &str = "task:status-row";
const INITIAL: &str = "Successful fs_write suffices for this output contract.";
const REVISED: &str = "Verify the independently read output after fs_write.";
const READY: &str = r#"[{"status":"ready"}]"#;
const WRONG: &str = r#"[{"status":"other"}]"#;

fn call(id: &str, name: &str, input: Value) -> Message {
    Message::assistant(vec![Block::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    }])
}

fn result(request: &CompletionRequest, id: &str) -> Option<String> {
    request
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .find_map(|block| match block {
            Block::ToolResult {
                tool_use_id,
                content,
                is_error: false,
            } if tool_use_id == id => Some(content.clone()),
            _ => None,
        })
}

fn packet(text: &str) -> Result<Value> {
    if text.starts_with('{') {
        return Ok(serde_json::from_str(text)?);
    }
    let body = text
        .split_once("\n---\n")
        .context("missing native memory wrapper")?
        .1
        .strip_suffix("\n</untrusted-content>")
        .context("incomplete native memory wrapper")?;
    Ok(serde_json::from_str(body)?)
}

fn records(packet: &Value) -> Result<&[Value]> {
    packet["memory"]["records"]
        .as_array()
        .map(Vec::as_slice)
        .context("missing memory records")
}

fn references(packet: &Value) -> Result<Vec<RevisionRef>> {
    records(packet)?
        .iter()
        .map(|hit| Ok(serde_json::from_value(hit["reference"].clone())?))
        .collect()
}

enum Task {
    Independent(&'static str),
    Reuse,
    Inspect,
}

struct Authored {
    task: Task,
    packet: Arc<Mutex<Option<Value>>>,
}

#[async_trait::async_trait]
impl Provider for Authored {
    fn id(&self) -> &str {
        "authored-integrated"
    }
    fn default_model(&self) -> &str {
        "authored-no-model"
    }
    fn local(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
        _: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        let message = if result(request, "write").is_some() {
            Message::assistant(vec![Block::text("OK")])
        } else if let Some(text) = result(request, "recall") {
            let response = packet(&text)?;
            let message = match self.task {
                Task::Reuse => {
                    let concept = records(&response)?
                        .iter()
                        .find(|hit| {
                            hit["record"]["kind"] == "concept"
                                && hit["record"]["data"]["name"] == NAME
                        })
                        .context("missing provisional concept before writing")?;
                    let data = &concept["record"]["data"];
                    ensure!(
                        data["status"] == "provisional" && data["principle"] == INITIAL,
                        "wrong provisional concept"
                    );
                    let source = &data["support"][0]["evidence"];
                    let observation = records(&response)?
                        .iter()
                        .find(|hit| &hit["reference"] == source)
                        .context("concept support absent from recall")?;
                    let observation: ArtifactObservation = serde_json::from_str(
                        observation["record"]["data"]["text"]
                            .as_str()
                            .context("missing original artifact text")?,
                    )?;
                    let output = observation
                        .artifact_text
                        .context("supporting artifact unavailable")?;
                    call(
                        "write",
                        "fs_write",
                        json!({"path":"output.json","content":output}),
                    )
                }
                Task::Inspect => Message::assistant(vec![Block::text("Reviewed.")]),
                Task::Independent(_) => anyhow::bail!("independent session consulted memory"),
            };
            *self.packet.lock().unwrap() = Some(response);
            message
        } else {
            match self.task {
                Task::Independent(output) => call(
                    "write",
                    "fs_write",
                    json!({"path":"output.json","content":output}),
                ),
                Task::Reuse | Task::Inspect => {
                    call("recall", "goal_context", json!({"memory":{"query":NAME}}))
                }
            }
        };
        let stop_reason = if message
            .content
            .iter()
            .any(|b| matches!(b, Block::ToolUse { .. }))
        {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };
        Ok(CompletionResponse {
            message,
            stop_reason,
            usage: Usage::default(),
            refusal: None,
            model: self.default_model().into(),
            malformed_tool_args: 0,
        })
    }
}

mod support;

struct Fixture {
    root: PathBuf,
    database: PathBuf,
    authority: PathBuf,
    request: NativeSessionRequest,
    key: Option<PathBuf>,
}

impl Fixture {
    fn new(encrypted: bool) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let root = support::tmpdir("integrated-native");
        let db_dir = root.join("data");
        fs::create_dir(&db_dir)?;
        let database = db_dir.join("test.memory-prototype.sqlite");
        let key = if encrypted {
            let owner = root.join("owner");
            fs::DirBuilder::new().mode(0o700).create(&owner)?;
            Some(owner.join("memory.key"))
        } else {
            None
        };
        let authority = root.join("authority");
        drop(LocalAuthority::create(&authority)?);
        let request = NativeSessionRequest {
            prompt: "Save the status row in output.json.".into(),
            files: BTreeMap::new(),
            criterion: ConfirmedCriterion {
                goal: GOAL.into(),
                goal_text: "Write the status row.".into(),
                criterion_text: "output.json must contain one row with status ready.".into(),
                criterion_revision: "a".repeat(64),
                confirmation_digest: "b".repeat(64),
            },
            expected_rows: vec![BTreeMap::from([("status".into(), "ready".into())])],
            domain: "synthetic-native-lifecycle".into(),
            scopes: ["synthetic:status-row".into()].into(),
            account: "synthetic".into(),
            interpretation_context_id: None,
        };
        let storage = if let Some(key) = &key {
            StorageAccess::generate_key_file(
                key,
                &KeyFilePolicy::new(&[db_dir, authority.clone()])?,
            )?
        } else {
            StorageAccess::plaintext_fixture()
        };
        drop(Memory::create_supported_situated_with_storage(
            &database,
            &request.domain,
            storage,
        )?);
        Ok(Self {
            root,
            database,
            authority,
            request,
            key,
        })
    }

    fn storage(&self) -> Result<StorageAccess> {
        match &self.key {
            Some(key) => StorageAccess::load_key_file(
                key,
                &KeyFilePolicy::new(&[
                    self.database.parent().unwrap().to_path_buf(),
                    self.authority.clone(),
                ])?,
            ),
            None => Ok(StorageAccess::plaintext_fixture()),
        }
    }

    fn open(&self) -> Result<Memory> {
        Ok(Memory::open_with_storage(&self.database, self.storage()?)?
            .with_source_guard(Arc::new(LocalAuthority::open(&self.authority)?)))
    }

    fn access(&self) -> Access {
        Access {
            domain: self.request.domain.clone(),
            scopes: self.request.scopes.clone(),
        }
    }

    async fn run(&self, name: &str, task: Task) -> Result<(TaskReport, Option<Value>)> {
        let root = self.root.join(name);
        let authority = LocalAuthority::open(&self.authority)?;
        let packet = Arc::new(Mutex::new(None));
        let provider = Authored {
            task,
            packet: packet.clone(),
        };
        let factory = |_| async { Ok(Box::new(provider) as Box<dyn Provider>) };
        let cancel = CancellationToken::new();
        let report = if self.key.is_some() {
            run_native_session_with_storage(
                &self.database,
                self.storage()?,
                &authority,
                &self.request,
                &root,
                factory,
                &cancel,
            )
            .await?
        } else {
            run_native_session(
                &self.database,
                &authority,
                &self.request,
                &root,
                factory,
                &cancel,
            )
            .await?
        };
        ensure!(
            report.run_error.is_none(),
            "native run failed: {:?}",
            report.run_error
        );
        ensure!(
            report.capture_error.is_none(),
            "native capture failed: {:?}",
            report.capture_error
        );
        let result = packet.lock().unwrap().take();
        Ok((report, result))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn capture_refs(report: &TaskReport) -> Result<impl Iterator<Item = &RevisionRef>> {
    let receipt = report
        .receipt
        .as_ref()
        .context("missing native capture receipt")?;
    Ok(std::iter::once(&receipt.projection).chain(&receipt.originals))
}

fn projection(db: &Memory, report: &TaskReport, access: &Access) -> Result<Value> {
    let receipt = report
        .receipt
        .as_ref()
        .context("missing native capture receipt")?;
    let Some(Record::Evidence(value)) = db.read(&receipt.projection, access)? else {
        anyhow::bail!("missing capture projection");
    };
    Ok(serde_json::from_str(&value.text)?)
}

fn episode(
    db: &Memory,
    report: &TaskReport,
    access: &Access,
    expected: &str,
    verdict: CheckVerdict,
) -> Result<(Value, Vec<ConceptExcerpt>)> {
    assert_eq!(fs::read_to_string(&report.artifact)?, expected);
    assert_eq!(report.check.verdict, verdict);
    let projection = projection(db, report, access)?;
    assert_eq!(projection["criterion"]["goal"], GOAL);
    assert!(projection["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|receipt| receipt["call_id"] == "write"
            && receipt["tool_name"] == "fs_write"
            && receipt["status"] == "Succeeded"));
    let descriptors = projection["originals"]
        .as_array()
        .context("missing original descriptors")?;
    let write = descriptors
        .iter()
        .filter(|value| value["kind"] == "ActionInvocation")
        .map(|descriptor| serde_json::from_value::<RevisionRef>(descriptor["reference"].clone()))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(|reference| match db.read(&reference, access)? {
            Some(Record::Evidence(value)) => {
                Ok(serde_json::from_str::<ActionInvocation>(&value.text)?)
            }
            _ => anyhow::bail!("missing action invocation"),
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .find(|invocation| invocation.call_id == "write")
        .context("missing captured write invocation")?;
    assert_eq!(write.tool_name, "fs_write");
    assert_eq!(
        write.input,
        json!({"path":"output.json", "content":expected})
    );
    let mut excerpts = Vec::new();
    for kind in ["ArtifactObservation", "ToolResult"] {
        let original = descriptors
            .iter()
            .find(|value| value["kind"] == kind)
            .context("missing original evidence")?;
        let evidence = serde_json::from_value(original["reference"].clone())?;
        assert!(report
            .receipt
            .as_ref()
            .unwrap()
            .originals
            .contains(&evidence));
        let Some(Record::Evidence(value)) = db.read(&evidence, access)? else {
            anyhow::bail!("missing original revision");
        };
        let quote = if kind == "ArtifactObservation" {
            let observation: ArtifactObservation = serde_json::from_str(&value.text)?;
            assert_eq!(observation.content_status, ContentStatus::Complete);
            assert_eq!(observation.artifact_text.as_deref(), Some(expected));
            assert_eq!(observation.verdict, verdict);
            assert_eq!(observation.artifact_digest, report.check.artifact_digest);
            match verdict {
                CheckVerdict::Passed => r#""verdict":"Passed""#,
                CheckVerdict::Failed => r#""verdict":"Failed""#,
                _ => anyhow::bail!("unexpected fixture verdict"),
            }
        } else {
            assert!(value.text.contains("output.json"));
            "wrote"
        };
        let start = value
            .text
            .find(quote)
            .context("quote missing from original")?;
        excerpts.push(ConceptExcerpt {
            evidence,
            field: original["pointer"].as_str().unwrap().into(),
            start,
            end: start + quote.len(),
            quote: quote.into(),
        });
    }
    Ok((projection, excerpts))
}

fn selections(excerpts: &[ConceptExcerpt]) -> Vec<Value> {
    excerpts
        .iter()
        .map(|e| json!({"source_id":e.evidence.revision,"field":e.field,"quote":e.quote}))
        .collect()
}

fn draft(principle: &str, support: &[ConceptExcerpt], opposition: &[ConceptExcerpt]) -> Value {
    json!({"name":NAME,"principle":principle,"applicability":"This confirmed status row output contract.",
        "support":selections(support),"counterexamples":selections(opposition)})
}

#[tokio::test]
async fn authored_native_recall_revision_and_withdrawal_preserve_lineage() -> Result<()> {
    authored_lifecycle(false).await
}

#[tokio::test]
async fn full_native_artifact_citations_fit_revised_context_without_losing_lineage() -> Result<()> {
    authored_lifecycle(true).await
}

async fn authored_lifecycle(full_citations: bool) -> Result<()> {
    let fixture = Fixture::new(false)?;
    let access = fixture.access();
    let (first, _) = fixture.run("first", Task::Independent(READY)).await?;
    let (second, _) = fixture.run("second", Task::Independent(READY)).await?;
    let mut db = fixture.open()?;
    let (a, mut a_sources) = episode(&db, &first, &access, READY, CheckVerdict::Passed)?;
    let (b, mut b_sources) = episode(&db, &second, &access, READY, CheckVerdict::Passed)?;
    assert_ne!(a["identity"]["session_id"], b["identity"]["session_id"]);
    assert_eq!(a["criterion"], b["criterion"]);
    assert_eq!(a["consulted"], json!([]));
    assert_eq!(b["consulted"], json!([]));
    if full_citations {
        for source in [&mut a_sources[0], &mut b_sources[0]] {
            full_original_quote(&db, source, &access)?;
        }
    }
    let support = [a_sources[0].clone(), b_sources[0].clone()];
    let source_refs = a_sources
        .iter()
        .chain(&b_sources)
        .map(|e| e.evidence.clone())
        .collect::<Vec<_>>();
    let prepared = db.prepare_native_concept(&source_refs, &db.snapshot()?, &access)?;
    let concept = db.retain_native_concept(
        prepared,
        &draft(INITIAL, &support, &[]).to_string(),
        &access,
        None,
    )?;
    drop(db);

    let (reuse, packet) = fixture.run("reuse", Task::Reuse).await?;
    let packet = packet.context("native reuse skipped recall")?;
    assert_eq!(packet["memory"]["situation"]["goal"], GOAL);
    let visible = references(&packet)?;
    assert!(visible.contains(&concept));
    assert!(capture_refs(&first)?
        .chain(capture_refs(&second)?)
        .all(|reference| visible.contains(reference)));
    assert_eq!(packet["memory"]["budget_omitted"], false);
    assert!(reuse.taint.private && reuse.taint.untrusted);
    let db = fixture.open()?;
    let (influenced, _) = episode(&db, &reuse, &access, READY, CheckVerdict::Passed)?;
    let consulted: Vec<RevisionRef> = serde_json::from_value(influenced["consulted"].clone())?;
    assert!(visible
        .iter()
        .all(|reference| consulted.contains(reference)));
    drop(db);

    let (wrong, _) = fixture
        .run("counterexample", Task::Independent(WRONG))
        .await?;
    let mut db = fixture.open()?;
    let (contradiction, mut contradicting_sources) =
        episode(&db, &wrong, &access, WRONG, CheckVerdict::Failed)?;
    assert_eq!(a["criterion"], contradiction["criterion"]);
    assert_eq!(contradiction["consulted"], json!([]));
    assert_ne!(
        a["identity"]["session_id"],
        contradiction["identity"]["session_id"]
    );
    assert_ne!(
        b["identity"]["session_id"],
        contradiction["identity"]["session_id"]
    );
    if full_citations {
        full_original_quote(&db, &mut contradicting_sources[0], &access)?;
    }
    let opposition = &contradicting_sources[..if full_citations { 2 } else { 1 }];
    let prepared =
        db.prepare_native_concept_rumination(&concept, opposition, &db.snapshot()?, &access)?;
    let reply =
        json!({"outcome":"revised_provisional","draft":draft(REVISED, &support, opposition)});
    let RuminationOutcome::RevisedProvisional { reference: revised } =
        db.finish_native_concept_rumination(prepared, &reply.to_string(), &access)?
    else {
        anyhow::bail!("authored revision was not retained");
    };
    assert_eq!(revised.object, concept.object);
    assert_ne!(revised.revision, concept.revision);
    drop(db);

    let db = fixture.open()?;
    let Some(Record::Concept(stored)) = db.read(&revised, &access)? else {
        anyhow::bail!("revision absent after reopening");
    };
    assert!(matches!(stored.status, ConceptStatus::Provisional));
    assert_eq!(stored.support.len(), 2);
    assert_eq!(
        stored.counterexamples.len(),
        if full_citations { 2 } else { 1 }
    );
    if full_citations {
        let expanded = db.context(
            &mecha_core::memory::context::ContextRequest {
                query: NAME.into(),
                bindings: vec![],
            },
            &mecha_core::memory::context::ContextPolicy {
                access: access.clone(),
                recall: mecha_core::memory::RecallQuery {
                    text: NAME.into(),
                    known_at: db.snapshot()?,
                    account: Some("synthetic".into()),
                    session_id: None,
                    occurred_after: None,
                    occurred_before: None,
                    limit: 3,
                    neighbors: 0,
                    max_bytes: 100_000,
                },
                valid_at: None,
            },
            &mecha_core::memory::context::Situation {
                event: fixture.request.prompt.clone(),
                goal: GOAL.into(),
                needs: None,
            },
        )?;
        let mut canonical = serde_json::to_value(&expanded)?;
        canonical["records"] = serde_json::to_value(&expanded.records)?;
        let canonical_bytes = serde_json::to_vec(&canonical)?.len();
        eprintln!(
            "full citation canonical bytes={canonical_bytes}; reader bytes={}; manifest={}",
            serde_json::to_vec(&expanded)?.len(),
            serde_json::to_string(
                &expanded
                    .records
                    .iter()
                    .map(|h| &h.reference)
                    .collect::<Vec<_>>()
            )?
        );
        assert!(
            canonical_bytes > 20_000,
            "regression must exceed the unchanged packet cap"
        );
    }
    let (revised_capture, packet) = fixture.run("revised-read", Task::Inspect).await?;
    let packet = packet.context("native revised recall absent")?;
    let visible = references(&packet)?;
    assert!(visible.contains(&revised));
    assert!(!visible.contains(&concept));
    let expected = capture_refs(&first)?
        .chain(capture_refs(&second)?)
        .chain(capture_refs(&wrong)?)
        .chain(std::iter::once(&revised))
        .map(|r| (&r.object, &r.revision))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        visible
            .iter()
            .map(|r| (&r.object, &r.revision))
            .collect::<std::collections::BTreeSet<_>>(),
        expected
    );
    assert_eq!(
        visible.len(),
        expected.len(),
        "overlapping bundles must not duplicate revisions"
    );
    assert!(serde_json::to_vec(&packet["memory"])?.len() <= 20_000);
    let recaptured = projection(&db, &revised_capture, &access)?;
    assert_eq!(
        serde_json::from_value::<Vec<RevisionRef>>(recaptured["consulted"].clone())?,
        visible
    );
    for original in &revised_capture
        .receipt
        .as_ref()
        .context("missing recapture")?
        .originals
    {
        let Some(Record::Evidence(source)) = db.read(original, &access)? else {
            anyhow::bail!("recapture original unavailable")
        };
        assert!(!source.text.contains("native_capture_reader_v1"));
    }
    assert!(stored
        .consulted
        .iter()
        .all(|reference| visible.contains(reference)));
    assert!(capture_refs(&first)?
        .chain(capture_refs(&second)?)
        .chain(capture_refs(&wrong)?)
        .all(|reference| visible.contains(reference)));
    assert_eq!(packet["memory"]["budget_omitted"], false);
    let data = records(&packet)?
        .iter()
        .find(|hit| hit["record"]["kind"] == "concept")
        .context("missing revised concept")?;
    let recalled: Concept = serde_json::from_value(data["record"]["data"].clone())?;
    assert_eq!(recalled.principle, REVISED);
    assert_eq!(selections(&recalled.support), selections(&support));
    assert_eq!(
        selections(&recalled.counterexamples),
        selections(opposition)
    );

    for reference in capture_refs(&reuse)? {
        assert!(db.read(reference, &access)?.is_some());
    }
    let authority = LocalAuthority::open(&fixture.authority)?;
    authority.withdraw(
        a["binding"]["locator"]
            .as_str()
            .context("missing source locator")?,
    )?;
    assert!(db.read(&revised, &access)?.is_none());
    for reference in capture_refs(&reuse)? {
        assert!(db.read(reference, &access)?.is_none());
    }
    let (_, packet) = fixture.run("after-withdrawal", Task::Inspect).await?;
    assert!(
        !references(&packet.context("missing post-withdrawal memory packet")?)?.contains(&revised)
    );
    Ok(())
}

fn full_original_quote(db: &Memory, excerpt: &mut ConceptExcerpt, access: &Access) -> Result<()> {
    let Some(Record::Evidence(original)) = db.read(&excerpt.evidence, access)? else {
        anyhow::bail!("missing original full citation");
    };
    excerpt.start = 0;
    excerpt.end = original.text.len();
    excerpt.quote = original.text;
    Ok(())
}

#[path = "native_concept_appraisal/appraisal.rs"]
mod concept_appraisal;
