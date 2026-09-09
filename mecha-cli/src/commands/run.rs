//! `mecha run` — one task, one answer.

use crate::{render, setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::message::{Message, StopReason};
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use std::io::{IsTerminal, Read};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// The task. Omit it, or pass `-`, to read from stdin.
    pub prompt: Option<String>,

    /// Emit a single JSON object instead of prose. Implies --quiet.
    #[arg(long)]
    pub json: bool,

    /// Print only the answer — no tool narration.
    #[arg(long)]
    pub quiet: bool,

    /// Wait for the whole answer instead of streaming it.
    #[arg(long)]
    pub no_stream: bool,

    /// Continue a saved session by id or unique prefix.
    #[arg(long)]
    pub resume: Option<String>,

    /// Confirm what this run serves (task:id, project:id, charter:id or setpoint:id).
    /// Overrides the saved anchor when resuming; omission preserves it.
    #[arg(long, value_name = "KIND:ID")]
    pub goal: Option<mecha_core::goal::GoalRef>,

    /// Owner-authored JSON fixture for isolated artifact validation of later mismatches.
    #[arg(long, conflicts_with_all = ["resume", "no_session", "images"])]
    pub mismatch_case: Option<std::path::PathBuf>,

    /// Don't write a transcript.
    #[arg(long)]
    pub no_session: bool,

    /// Put an image in front of the model. Repeatable.
    ///
    /// The terminal's counterpart to attaching a screenshot in Slack. Not a
    /// path the model is told about — `fs_read` already does that and cannot
    /// read a PNG — but the pixels themselves, on the user turn.
    ///
    /// Resolved against the working directory rather than the run's
    /// workspace, and deliberately: this is the *user* handing something
    /// over, exactly like typing a prompt, and the path jail exists to bound
    /// what the **model** can reach. The same reasoning `/send <path>` uses
    /// one direction over.
    #[arg(long = "image", value_name = "PATH")]
    pub images: Vec<std::path::PathBuf>,
}

fn confirm_goal(
    convo: &mut mecha_core::agent::Conversation,
    goal: Option<mecha_core::goal::GoalRef>,
    session: Option<&Session>,
) -> Result<()> {
    if let Some(goal) = goal {
        if let Some(session) = session {
            session.append(&Record::GoalAnchor {
                goal: Some(goal.clone()),
            })?;
        }
        convo.goal_anchor = Some(goal);
    }
    Ok(())
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let prompt = read_prompt(args.prompt.as_deref())?;
    anyhow::ensure!(!prompt.trim().is_empty(), "no prompt given");

    // Nothing can answer an approval prompt when output is being piped or
    // parsed, so those runs use the configured permission mode instead.
    let interactive = std::io::stdin().is_terminal() && !args.json;
    let opts = GlobalOpts {
        surface: Some(mecha_core::session::SessionKind::Run),
        ..global.clone()
    };
    let mut prepared = setup::prepare(&opts, interactive).await?;

    let mismatch_case = args
        .mismatch_case
        .as_ref()
        .map(|p| mecha_core::mismatch::ArtifactCase::load(p))
        .transpose()?;
    if let Some(case) = &mismatch_case {
        case.bind(&prompt, args.goal.as_ref(), &prepared.workspace)?;
        mecha_core::mismatch::validate_recording(&RunConfig::of(
            &prepared.agent,
            &prepared.config,
            &prepared.provider_name,
            &prepared.levers_off,
            Some(&prepared.rules),
        ))?;
    }
    let session_dir = Session::default_dir()?;
    let mut convo = mecha_core::agent::Conversation::new();
    let mut session = None;

    if let Some(id) = &args.resume {
        let path = Session::find(&session_dir, id)?;
        let (meta, prior) = Session::load(&path)?;
        // D15, as in the other three resume paths.
        if let Some(todo) = &prepared.todo {
            let ws = prepared.agent.context().tools.workspace.clone();
            todo.rehydrate(&ws, &prior.messages);
        }
        convo = prior;
        session = Some(Session { meta, path });
    } else if !args.no_session {
        session = Some(Session::create(
            &session_dir,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: prepared.provider_name.clone(),
                model: prepared.model.clone(),
                workspace: prepared.workspace.clone(),
                title: Some(first_words(&prompt)),
                kind: Some(mecha_core::session::SessionKind::Run),
            },
        )?);
    }

    confirm_goal(&mut convo, args.goal, session.as_ref())?;

    // Written on create *and* on resume: a session picked up under different
    // flags is exactly the case this record exists to catch.
    if let Some(s) = &session {
        // Only a resumed run: a fresh one-shot's record is empty until the
        // run ends, so recall would be a dead spec in its prompt. Before the
        // config record, which captures the tool list for replay.
        if args.resume.is_some() {
            setup::register_recall(&mut prepared.agent, s);
        }
        let mut recorded = RunConfig::of(
            &prepared.agent,
            &prepared.config,
            &prepared.provider_name,
            &prepared.levers_off,
            Some(&prepared.rules),
        );
        recorded.mismatch_case = mismatch_case.clone();
        s.append(&Record::Config(recorded))?;
        // Staged outbox items point back at the session that drafted them.
        if let Some(route) = &prepared.agent.context().outbox {
            route.set_session_id(&s.meta.id);
        }
        // One-shots are their own producer, `run` — addressable, though
        // rarely addressed. `run` does not set `global_config_only`, so its
        // resolved inbound default is Hold whether stdin is a terminal or a
        // pipe: a scripted `run --json` must not fold a stray message into a
        // task it has nothing to do with. Only the trigger runner accepts.
        if let Some(mb) = &prepared.mailbox {
            mb.attach("run", &s.meta.id);
        }
    }

    // Read before the run rather than lazily: a path that does not exist, or
    // an image too large to send, is a mistake the person can fix now — and
    // failing here costs nothing, where failing mid-run costs the turn.
    let mut images = Vec::new();
    for path in &args.images {
        match mecha_core::image::block_from_path(path)? {
            Some(block) => images.push(block),
            None => anyhow::bail!(
                "{} is not an image this can send (png, jpeg, gif, webp)",
                path.display()
            ),
        }
    }
    // Said once, at the point it can still be acted on. The alternative is a
    // model that answers about a file it was never shown, which is the exact
    // failure this whole path was built to end.
    if !images.is_empty() && !prepared.agent.vision() {
        eprintln!(
            "warning: {} cannot see images, so {} will arrive as a line of text naming the \
             file. Check `[providers.*] vision` and whether the server was started with \
             --mmproj.",
            prepared.model,
            if images.len() == 1 { "it" } else { "they" },
        );
    }

    // Text first, images after — the order both provider families document.
    let user = if images.is_empty() {
        Message::user(&prompt)
    } else {
        let mut content = vec![mecha_core::message::Block::text(&prompt)];
        content.extend(images);
        Message {
            harness: false,
            planning: None,
            tool_provenance: Default::default(),
            role: mecha_core::message::Role::User,
            content,
        }
    };
    convo.push(user.clone());
    if let Some(s) = &session {
        s.append(&Record::Message(user))?;
    }
    // Exactly what the file holds now, for the post-run reconcile: a run
    // that only appended gets its tail appended, one that rewrote history
    // (compaction) gets a rewrite record.
    let recorded = convo.messages.clone();

    let quiet = args.quiet || args.json;
    let events = if args.no_stream || args.json {
        None
    } else {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = render::spawn(
            rx,
            render::RenderOpts {
                verbose: global.verbose,
                quiet,
            },
        );
        Some((tx, handle))
    };

    let result = crate::interrupt::run_interruptible(
        &prepared.agent,
        prepared.agent.context(),
        &mut convo,
        events.as_ref().map(|(tx, _)| tx.clone()),
    )
    .await;

    // Closing the sender ends the render task; wait so its output lands before
    // anything we print below.
    if let Some((tx, handle)) = events {
        drop(tx);
        let _ = handle.await;
    }

    if let Some(s) = &session {
        // Before the result is inspected, so a run that died mid-flight still
        // leaves its turns on disk. `?` first is how a fatal 400 used to leave
        // a three-line transcript of a three-minute benchmark trial — and the
        // failed runs are precisely the transcripts that get read.
        s.record_run(&recorded, &convo)?;
        // Taint too, and for the same reason `chat` and `tui` record it: it
        // cannot be recovered by reading the transcript back, because it keys
        // off *provenance* and the transcript stores only content. `run`
        // supports `--resume`, so without this, resuming a one-shot that had
        // read a hostile page hands the model that page with the interlock
        // disarmed — the exact hole that was closed for the other two
        // front-ends and left open here.
        s.append(&Record::Taint(convo.taint))?;
    }

    let outcome = result?;
    if let Some(s) = &session {
        if let Some(case) = &mismatch_case {
            if matches!(
                outcome.stop_cause,
                mecha_core::agent::StopCause::Completed
                    | mecha_core::agent::StopCause::MaxTurns
                    | mecha_core::agent::StopCause::OutputTokenBudget
            ) && !outcome.tool_calls.iter().any(|c| c.denied || c.staged)
                && outcome.blocked_sends == 0
            {
                append_criterion_feedback(s, case, &prepared.workspace, convo.taint)?;
            }
        }
        s.append(&Record::Summary {
            usage: outcome.usage.clone(),
            turns: outcome.turns,
        })?;
        s.record_outcome(&outcome)?;
        if let Some(mb) = &prepared.mailbox {
            mb.detach(&s.meta.id);
        }
    }

    if args.json {
        let value = result_json(
            &outcome,
            &prepared.model,
            &prepared.provider_name,
            session.as_ref().map(|s| s.meta.id.as_str()),
        );
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if args.no_stream {
        println!("{}", outcome.text);
    }

    if !quiet && !args.json {
        if let Some(s) = &session {
            eprintln!("\nsession {}", s.meta.id);
        }
    }

    // Before the exit-code match below: those paths leave the process.
    if let Some(s) = &session {
        let cx = prepared.agent.context();
        cx.hooks
            .session_end(&s.meta.id, &s.path, &cx.tools.workspace)
            .await;
    }

    // `process::exit` skips destructors. Finish recording and session hooks
    // first, then release the agent registry and its retained MCP clients so
    // refusal/no-output exits also launch process and container cleanup.
    drop(prepared);

    // Distinct codes so a script can tell "the model refused" from "it
    // produced nothing" from "everything worked". Exhaustion alone is *not*
    // a failure code: a run that hit its turn or token ceiling still answered
    // and still left its work on disk, and callers that grade the artifact
    // treat non-zero as "the agent crashed" — on the 2026-08-07
    // Terminal-Bench subset, a MaxTurns trial was recorded as an agent error
    // while its verifier scored the work 1.0. A script that cares which
    // ceiling stopped the run has `--json`'s `stop_cause`; the exit code
    // answers the only question a caller can't get elsewhere: is there an
    // answer at all.
    match outcome.stop_reason {
        StopReason::Refusal => std::process::exit(2),
        _ if outcome.stop_cause == mecha_core::agent::StopCause::NoOutput => std::process::exit(3),
        _ => Ok(()),
    }
}

fn append_criterion_feedback(
    session: &Session,
    case: &mecha_core::mismatch::ArtifactCase,
    workspace: &std::path::Path,
    taint: mecha_core::agent::Taint,
) -> Result<()> {
    let steps = case.criterion_feedback(workspace)?;
    if !steps.is_empty() {
        let mut message = Message::user("Harness observations of owner-bound task criteria.");
        message.harness = true;
        message.planning = Some(mecha_core::planning::Feedback {
            steps,
            ..Default::default()
        });
        session.append(&Record::Message(message))?;
        session.append(&Record::Taint(taint))?;
    }
    Ok(())
}

fn read_prompt(arg: Option<&str>) -> Result<String> {
    match arg {
        Some("-") | None => {
            if std::io::stdin().is_terminal() && arg.is_none() {
                anyhow::bail!("no prompt given (pass one as an argument, or pipe it on stdin)");
            }
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("reading prompt from stdin")?;
            Ok(buf)
        }
        Some(text) => Ok(text.to_string()),
    }
}

/// A short label for `sessions list`.
fn first_words(prompt: &str) -> String {
    let flat = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 60 {
        format!("{}…", flat.chars().take(60).collect::<String>())
    } else {
        flat
    }
}

/// The `--json` object: a superset of [`mecha_core::batch::BatchResult`],
/// field for field and value for value, so a caller that drives runs as
/// processes — `mecha exp` — reads the result back through the same type the
/// batch runner produces and hands it to the same grader. The fields the
/// batch result does not have (model, provider, session) sit beside them.
/// `ok` and `error` follow the batch runner's own rendering: a refusal is a
/// *struct* on the outcome and a string on the result, and the first cut
/// emitted the struct, which made every refused trial unparseable and
/// silently dropped its episode from both arms of an experiment (found on
/// review). The round-trip is tested below rather than promised here.
pub(crate) fn result_json(
    outcome: &mecha_core::agent::RunOutcome,
    model: &str,
    provider: &str,
    session: Option<&str>,
) -> serde_json::Value {
    let refused = outcome.stop_reason == mecha_core::StopReason::Refusal;
    serde_json::json!({
        "id": "run",
        "ok": !outcome.exhausted && !refused && outcome.malformed_tool_args == 0,
        "error": outcome.refusal.as_ref().map(|r| {
            format!(
                "refused ({}): {}",
                r.category.clone().unwrap_or_else(|| "unspecified".into()),
                r.explanation.clone().unwrap_or_default()
            )
        }),
        "text": outcome.text,
        "stop_reason": outcome.stop_reason,
        "turns": outcome.turns,
        "exhausted": outcome.exhausted,
        "ended_on_failed_call": outcome.ended_on_failed_call,
        "stop_cause": outcome.stop_cause,
        "cost_usd": outcome.cost_usd,
        "refusal": outcome.refusal,
        "usage": outcome.usage,
        "usage_complete": outcome.usage_complete,
        "elapsed_ms": outcome.duration_secs.map(|s| (s * 1000.0) as u64).unwrap_or(0),
        "tool_calls": outcome.tool_calls,
        "malformed_tool_args": outcome.malformed_tool_args,
        "blocked_sends": outcome.blocked_sends,
        "compactions": outcome.compactions,
        "taint": outcome.taint,
        "meta": serde_json::Value::Null,
        "model": model,
        "provider": provider,
        "session": session,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::agent::{RunOutcome, StopCause};
    use mecha_core::message::{Refusal, Usage};
    use mecha_core::StopReason;

    #[test]
    fn guidance_opt_out_is_global_like_other_execution_switches() {
        use clap::Parser;
        for args in [
            ["mecha", "--no-goal-guidance", "run", "go"],
            ["mecha", "run", "--no-goal-guidance", "go"],
        ] {
            assert!(
                crate::Cli::try_parse_from(args)
                    .unwrap()
                    .global
                    .no_goal_guidance
            );
        }
    }

    #[test]
    fn explicit_goal_parses_and_preserves_or_overrides_a_resumed_anchor() {
        use clap::Parser;
        let cli =
            crate::Cli::try_parse_from(["mecha", "run", "--goal", "task:next", "go"]).unwrap();
        let crate::Command::Run(args) = cli.command else {
            panic!("run command")
        };
        let mut convo = mecha_core::agent::Conversation::new();
        convo.goal_anchor = Some("task:old".parse().unwrap());
        confirm_goal(&mut convo, None, None).unwrap();
        assert_eq!(convo.goal_anchor.as_ref().unwrap().to_string(), "task:old");
        confirm_goal(&mut convo, args.goal, None).unwrap();
        assert_eq!(convo.goal_anchor.as_ref().unwrap().to_string(), "task:next");
        assert!(crate::Cli::try_parse_from(["mecha", "run", "--goal", "invalid", "go"]).is_err());
    }

    /// The superset claim, measured: a refused, cut-off run whose last call
    /// failed reads back through `BatchResult` with every field it can
    /// carry non-defaulted. Fails on the first cut twice over — the refusal
    /// struct did not parse as `error`, and `ended_on_failed_call` was not
    /// emitted at all, so a case expecting it always failed under `exp`.
    #[test]
    fn the_json_result_is_a_batch_result_a_grader_can_read() {
        let outcome = RunOutcome {
            text: "no".into(),
            stop_reason: StopReason::Refusal,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 2,
                ..Usage::default()
            },
            turns: 3,
            refusal: Some(Refusal {
                category: Some("policy".into()),
                explanation: Some("not this".into()),
            }),
            exhausted: false,
            ended_on_failed_call: true,
            tool_calls: vec![mecha_core::agent::ToolCallTrace {
                name: "shell".into(),
                input: serde_json::json!({"command": "ls"}),
                is_error: true,
                denied: false,
                unknown: false,
                staged: false,
            }],
            malformed_tool_args: 1,
            blocked_sends: 2,
            taint: mecha_core::agent::Taint {
                private: true,
                untrusted: false,
            },
            homeostat: None,
            context_overflows: 0,
            boredom_notices: 0,
            step_escalations_attempted: 0,
            step_escalations_revised: 0,
            step_nulls: 0,
            step_reopens: 0,
            step_completions: 0,
            step_measured: 0,
            goal_anchor: None,
            goal_plan_writes: 0,
            goal_drift_writes: 0,
            goal_unnamed_writes: 0,
            stop_cause: StopCause::Completed,
            compactions: 1,
            cost_usd: None,
            usage_complete: true,
            duration_secs: Some(1.5),
        };
        let value = result_json(&outcome, "m", "local", Some("sess"));
        let back: mecha_core::batch::BatchResult = serde_json::from_value(value.clone()).unwrap();
        assert!(!back.ok, "a refusal is not ok");
        assert_eq!(back.error.as_deref(), Some("refused (policy): not this"));
        assert!(back.ended_on_failed_call);
        assert_eq!(back.tool_calls.len(), 1);
        assert!(back.tool_calls[0].is_error);
        assert_eq!(back.malformed_tool_args, 1);
        assert_eq!(back.blocked_sends, 2);
        assert_eq!(back.compactions, 1);
        assert!(back.taint.private);
        assert_eq!(back.elapsed_ms, 1500);
        assert_eq!(back.stop_cause, Some(StopCause::Completed));
        assert_eq!(back.turns, 3);
        assert_eq!(value["session"], "sess");
    }
}

#[cfg(test)]
mod criterion_recording_tests {
    use super::*;
    #[test]
    fn diagnostic_taint_is_recorded_and_unknown_context_cannot_promote_it() {
        use mecha_core::{
            agent::Taint,
            learning::{classify_origin, Origin},
        };
        let root = mecha_core::mismatch::Workspace::new().unwrap();
        let case:mecha_core::mismatch::ArtifactCase=serde_json::from_value(serde_json::json!({
            "prompt":"make answer", "goal":"task:test", "files":{}, "artifacts":{"answer.json":{"ok":true}},
            "criteria":{"result":{"artifact":"answer.json","pointer":"/ok"}}
        })).unwrap();
        for untrusted in [false, true] {
            let session = Session::create(
                root.path(),
                SessionMeta {
                    id: Session::new_id(),
                    created_at: chrono::Utc::now(),
                    provider: "scripted".into(),
                    model: "scripted".into(),
                    workspace: root.path().into(),
                    title: None,
                    kind: None,
                },
            )
            .unwrap();
            append_criterion_feedback(
                &session,
                &case,
                root.path(),
                Taint {
                    private: true,
                    untrusted,
                },
            )
            .unwrap();
            let t = Session::read(&session.path).unwrap();
            let interventions =
                mecha_core::learning::extract_mismatches(&t.convo.messages, &t.outcome_positions);
            assert_eq!(interventions.len(), 1);
            assert_eq!(
                classify_origin(t.taint_timeline.covering(interventions[0].at)),
                if untrusted {
                    Origin::Untrusted
                } else {
                    Origin::Clean
                }
            );
            assert_eq!(
                t.convo.messages[0].text(),
                "Harness observations of owner-bound task criteria."
            );
            assert!(t.convo.messages[0].harness);
        }
    }
}
