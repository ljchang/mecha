//! The half of replay that runs: recorded tools, and the loop that drives them.
//!
//! [`crate::replay`] is deliberately pure — extraction and diffing are where
//! the interesting mistakes live, and they are unit-testable. This module is
//! the impure remainder: a [`Registry`] of tools that answer from a recording
//! instead of executing, and a driver that feeds the recorded user turns to an
//! agent and collects what it did differently.
//!
//! Replaying against *live* tools would re-read a filesystem and a web that
//! have both moved since the recording, so a divergence would tell you nothing
//! about the harness or the model. Answering from the recording isolates the
//! variable: same turns, same tool results, and the only thing left that can
//! differ is what the model chose to do with them.
//!
//! What a replayed result is not: provenance. The transcript does not record
//! which results actually came from outside, so replayed outputs are returned
//! without the `external` marking and the replay's taint may be *less* armed
//! than the recording's was. Refusals the interlock produced at record time
//! were recorded as results, so they replay verbatim regardless.

use crate::agent::{Agent, Conversation, RunContext, ToolCallTrace};
use crate::message::Message;
use crate::replay::{diff, Divergence, RecordedCall, Trajectory};
use crate::tool::{Capabilities, Registry, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// What to do when the replay departs from the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnDivergence {
    /// Stop the run. After a divergence every later recorded result answers a
    /// question nobody asked, so this is the honest default.
    #[default]
    Stop,
    /// Stop, and let the caller treat any divergence as a failure. The run
    /// behaves exactly like [`OnDivergence::Stop`]; the difference is policy
    /// the caller applies to the report.
    Error,
    /// Abandon the recording and fall through to the live tools. The
    /// comparison is over at that point; what remains is a fresh run seeded
    /// with the recorded prefix.
    Live,
}

/// The shared cursor over the recording. One per replay, shared by every
/// [`ReplayTool`] in the registry, because the recording is one sequence — the
/// order calls happen in *is* the trajectory, whichever tool they went to.
struct ReplayState {
    calls: Vec<RecordedCall>,
    cursor: usize,
    /// Set at the first structural divergence. From then on the recording has
    /// nothing truthful left to say.
    dead: bool,
}

/// What one call decided to do, resolved under the lock and acted on after it.
enum Action {
    Recorded(String, bool),
    Refuse(String),
    Live,
}

/// A tool that answers from the recording.
///
/// Wraps the live tool rather than replacing it: the spec the model sees —
/// name, description, schema — is the live one, which is exactly right, because
/// a changed description *is* part of what a replay measures. The live tool is
/// only ever executed in [`OnDivergence::Live`] mode.
struct ReplayTool {
    inner: Arc<dyn Tool>,
    mode: OnDivergence,
    state: Arc<Mutex<ReplayState>>,
    cancel: CancellationToken,
}

impl ReplayTool {
    fn decide(&self, input: &Value) -> Action {
        let mut st = self.state.lock().unwrap();

        if st.dead {
            return match self.mode {
                OnDivergence::Live => Action::Live,
                _ => Action::Refuse(
                    "replay: the run has diverged from the recording; no recorded result \
                     exists for this call"
                        .into(),
                ),
            };
        }

        let Some(want) = st.calls.get(st.cursor) else {
            // The model kept going past the end of the recording.
            st.dead = true;
            return match self.mode {
                OnDivergence::Live => Action::Live,
                _ => {
                    self.cancel.cancel();
                    Action::Refuse(format!(
                        "replay: the recording ended after {} calls and has no result for \
                         this one; stopping",
                        st.calls.len()
                    ))
                }
            };
        };

        if want.name != self.inner.name() {
            let msg = format!(
                "replay: recorded call #{} was `{}`, not `{}`; stopping",
                st.cursor,
                want.name,
                self.inner.name()
            );
            st.dead = true;
            return match self.mode {
                OnDivergence::Live => Action::Live,
                _ => {
                    self.cancel.cancel();
                    Action::Refuse(msg)
                }
            };
        }

        // Same tool. Different arguments are *not* grounds to stop: a path
        // spelled differently is the same decision, and the final diff reports
        // every argument difference for the caller to judge. Returning the
        // recorded result for materially different arguments is the price of
        // not pretending to know which differences matter.
        let _ = input;
        let out = Action::Recorded(want.output.clone(), want.is_error);
        st.cursor += 1;
        out
    }
}

#[async_trait]
impl Tool for ReplayTool {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn input_schema(&self) -> Value {
        self.inner.input_schema()
    }
    fn read_only(&self) -> bool {
        self.inner.read_only()
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        // Decided under the lock, executed after it: a live call awaits, and a
        // std mutex must not be held across an await point.
        match self.decide(&input) {
            Action::Recorded(content, is_error) => Ok(ToolOutput {
                content,
                is_error,
                external: false,
            }),
            Action::Refuse(msg) => Ok(ToolOutput::err(msg)),
            Action::Live => self.inner.call(input, ctx).await,
        }
    }
}

/// Build a registry that offers the recorded tool surface and answers from the
/// recording.
///
/// `recorded_tools` is the tool list from the session's `RunConfig` — what the
/// model saw at record time. Every name must resolve in today's registry,
/// because the replay needs the live specs to offer (and the live tools to fall
/// through to in [`OnDivergence::Live`]). A tool that existed then and not now
/// is an error rather than a silent shrink of the surface: the model would be
/// replaying a different question.
/// Build the tool surface a replay offers, from the names the recording holds.
///
/// `surface_only` is consulted only in the modes where **nothing executes** —
/// [`OnDivergence::Stop`] and [`OnDivergence::Error`] — for names the live
/// registry cannot supply. The distinction it exists for: a recorded
/// tool is needed here to *describe itself* into the request — name,
/// description, schema — because the tool list is the front of the prompt, and
/// a replay offering a smaller toolbox is a different agent whose divergences
/// say nothing about the question being probed. Under `Stop` nothing is ever
/// executed (`Action::Live` is unreachable), so a stand-in that can describe
/// itself is a *faithful* surface rather than a fake one.
///
/// Under [`OnDivergence::Live`] tools genuinely run, and there a missing tool
/// must still be fatal — a stand-in would execute nothing while the recording
/// executed something. The gate is on the mode rather than on the caller's
/// good intentions, because an unconditional fallback silently changes what
/// `mecha replay` does.
///
/// **Which tools may be reconstructed is the caller's to decide, never this
/// module's.** Naming one here would put a front-end's registration policy in
/// core — the same rule that keeps `OutboxKind` config's to declare rather
/// than the tool's.
pub fn replay_registry(
    recorded_tools: &[String],
    live: &Registry,
    surface_only: Option<&Registry>,
    calls: Vec<RecordedCall>,
    mode: OnDivergence,
    cancel: CancellationToken,
) -> Result<Registry> {
    let state = Arc::new(Mutex::new(ReplayState {
        calls,
        cursor: 0,
        dead: false,
    }));
    let mut registry = Registry::new();
    for name in recorded_tools {
        // `Error` and not just `Stop`: the two run identically and differ only
        // in the policy the *caller* applies to the report, so gating on
        // `Stop` alone would leave one non-executing mode still bailing — a
        // guard that reads correct and is wrong for the one variant whose
        // difference is not behavioural.
        let executes = matches!(mode, OnDivergence::Live);
        let stand_in = (!executes)
            .then(|| surface_only.and_then(|r| r.get(name)))
            .flatten();
        let Some(tool) = live.get(name).or(stand_in) else {
            bail!(
                "recorded tool `{name}` is not available now, so the replay cannot offer \
                 the tool surface the model saw. Enable whatever provided it (an MCP \
                 server? a search backend?) and retry"
            );
        };
        registry.insert(Arc::new(ReplayTool {
            inner: Arc::clone(tool),
            mode,
            state: Arc::clone(&state),
            cancel: cancel.clone(),
        }));
    }
    Ok(registry)
}

/// What a replay produced, ready to be judged.
#[derive(Debug)]
pub struct ReplayReport {
    /// Every departure from the recording, in call order.
    pub divergences: Vec<Divergence>,
    /// The calls the replayed model actually made.
    pub replayed_calls: Vec<ToolCallTrace>,
    /// How many calls the recording holds, for "replayed M of N" reporting.
    pub recorded_calls: usize,
    /// User turns fed before the run ended.
    pub turns: usize,
    /// True when a divergence policy stopped the run before the turns ran out.
    pub stopped_early: bool,
    /// The replayed final answer, to set beside the recorded one.
    pub final_text: String,
    /// How the replayed episode went, in the same counters a live run
    /// records. This is what a candidate arm is compared on — the driver
    /// already had every outcome and used to keep only the calls and the
    /// text, which made the whole episode ungradeable by anything but a
    /// divergence diff.
    pub stats: crate::session::RunStats,
}

impl ReplayReport {
    /// Divergences that change what the model did, not how it spelled it.
    pub fn structural(&self) -> impl Iterator<Item = &Divergence> {
        self.divergences.iter().filter(|d| d.is_structural())
    }
}

/// Feed the recorded user turns to `agent` and diff what it did against the
/// recording.
///
/// The agent must have been built with a [`replay_registry`] sharing `cx`'s
/// cancellation token — that token is how a divergence stops the run at the
/// next safe point instead of burning turns against a dead recording.
///
/// Sequential on purpose, one conversation, one turn at a time: seeded sampling
/// only repeats when requests do not share a batch, so a replay that raced
/// anything else would diverge for reasons that are nobody's regression.
pub async fn drive(
    agent: &Agent,
    cx: &RunContext,
    trajectory: &Trajectory,
) -> Result<ReplayReport> {
    let mut convo = Conversation::new();
    let mut replayed: Vec<ToolCallTrace> = Vec::new();
    let mut final_text = String::new();
    let mut turns = 0;
    let mut stopped_early = false;
    // True until a turn says otherwise, as everywhere else that totals usage.
    let mut stats = crate::session::RunStats {
        usage_complete: true,
        ..Default::default()
    };

    for turn in &trajectory.turns {
        convo.push(Message::user(turn.clone()));
        let outcome = agent.run_in(cx, &mut convo, None).await?;
        stats.absorb(&outcome);
        replayed.extend(outcome.tool_calls);
        final_text = outcome.text;
        turns += 1;
        if cx.cancelled() {
            stopped_early = true;
            break;
        }
    }

    Ok(ReplayReport {
        divergences: diff(&trajectory.calls, &replayed),
        replayed_calls: replayed,
        recorded_calls: trajectory.calls.len(),
        turns,
        stopped_early,
        final_text,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Budget;
    use crate::config::{AgentConfig, PermissionMode};
    use crate::message::{Block, CompletionRequest, CompletionResponse, StopReason, Usage};
    use crate::provider::{Provider, StreamSink};
    use crate::tool::ModeApprover;
    use serde_json::json;

    struct EchoTool;
    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echo the `value` argument back."
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn read_only(&self) -> bool {
            true
        }
        async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
            Ok(ToolOutput::ok(format!(
                "live: {}",
                input.get("value").and_then(Value::as_str).unwrap_or("")
            )))
        }
    }

    /// A tool the live registry cannot build is served from `surface_only`
    /// **under `Stop` and nowhere else**.
    ///
    /// The case this exists for: `ask_user` is registered only by a front-end
    /// that owns a human, and it sits on the recorded surface of every
    /// interactive session — which is every session containing a steer. Before
    /// this, `replay_registry` bailed and so every steer and denial probe
    /// skipped, silently, on 246 of 408 sessions in the live store.
    #[test]
    fn a_surface_only_tool_fills_a_gap_under_stop_and_never_otherwise() {
        let live = Registry::new();
        let mut fallback = Registry::new();
        fallback.insert(Arc::new(OtherTool));
        let recorded = vec![OtherTool.name().to_string()];

        let stopped = replay_registry(
            &recorded,
            &live,
            Some(&fallback),
            Vec::new(),
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .expect("a stand-in describes the recorded surface under Stop");
        assert!(stopped.get(OtherTool.name()).is_some());

        // `Error` runs identically to `Stop` and must behave identically here.
        assert!(replay_registry(
            &recorded,
            &live,
            Some(&fallback),
            Vec::new(),
            OnDivergence::Error,
            CancellationToken::new(),
        )
        .is_ok());

        // Under the one mode that actually executes, a stand-in would run
        // nothing where the recording ran something. Still fatal.
        assert!(replay_registry(
            &recorded,
            &live,
            Some(&fallback),
            Vec::new(),
            OnDivergence::Live,
            CancellationToken::new(),
        )
        .is_err());

        // And with no fallback offered it is fatal even under Stop — the
        // behaviour every existing caller had, unchanged.
        assert!(replay_registry(
            &recorded,
            &live,
            None,
            Vec::new(),
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .is_err());
    }

    struct OtherTool;
    #[async_trait]
    impl Tool for OtherTool {
        fn name(&self) -> &str {
            "other"
        }
        fn description(&self) -> &str {
            "A second tool."
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn read_only(&self) -> bool {
            true
        }
        async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
            Ok(ToolOutput::ok("live: other"))
        }
    }

    fn live_registry() -> Registry {
        let mut r = Registry::new();
        r.insert(Arc::new(EchoTool));
        r.insert(Arc::new(OtherTool));
        r
    }

    fn recorded(name: &str, input: Value, output: &str) -> RecordedCall {
        RecordedCall {
            name: name.into(),
            input,
            output: output.into(),
            is_error: false,
        }
    }

    fn replay_reg(
        calls: Vec<RecordedCall>,
        mode: OnDivergence,
        cancel: &CancellationToken,
    ) -> Registry {
        replay_registry(
            &["echo".to_string(), "other".to_string()],
            &live_registry(),
            None,
            calls,
            mode,
            cancel.clone(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn matching_calls_replay_the_recorded_outputs_in_order() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![
                recorded("echo", json!({"value": "a"}), "first"),
                recorded("other", json!({}), "second"),
            ],
            OnDivergence::Stop,
            &cancel,
        );
        let ctx = ToolCtx::default();

        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({"value": "a"}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "first");
        assert!(!out.is_error);

        let out = reg
            .get("other")
            .unwrap()
            .call(json!({}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "second");
        assert!(!cancel.is_cancelled());
    }

    #[tokio::test]
    async fn a_recorded_error_replays_as_an_error() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![RecordedCall {
                name: "echo".into(),
                input: json!({}),
                output: "no such file".into(),
                is_error: true,
            }],
            OnDivergence::Stop,
            &cancel,
        );
        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({}), &ToolCtx::default())
            .await
            .unwrap();
        assert!(
            out.is_error,
            "the model must see the same failure it saw at record time"
        );
        assert_eq!(out.content, "no such file");
    }

    #[tokio::test]
    async fn a_different_tool_stops_the_run_and_kills_the_recording() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![
                recorded("echo", json!({}), "first"),
                recorded("echo", json!({}), "second"),
            ],
            OnDivergence::Stop,
            &cancel,
        );
        let ctx = ToolCtx::default();

        let out = reg
            .get("other")
            .unwrap()
            .call(json!({}), &ctx)
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(
            cancel.is_cancelled(),
            "a structural divergence must stop the run"
        );

        // The recording is dead: even the tool the recording expected gets
        // nothing now, rather than an answer to a question nobody asked.
        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({}), &ctx)
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("diverged"));
    }

    #[tokio::test]
    async fn running_past_the_end_of_the_recording_stops() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![recorded("echo", json!({}), "only")],
            OnDivergence::Stop,
            &cancel,
        );
        let ctx = ToolCtx::default();

        reg.get("echo")
            .unwrap()
            .call(json!({}), &ctx)
            .await
            .unwrap();
        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({}), &ctx)
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn different_arguments_still_replay_and_do_not_stop() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![recorded("echo", json!({"value": "a.md"}), "contents")],
            OnDivergence::Stop,
            &cancel,
        );
        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({"value": "./a.md"}), &ToolCtx::default())
            .await
            .unwrap();
        assert_eq!(out.content, "contents");
        assert!(
            !cancel.is_cancelled(),
            "argument differences are reported by the diff, not fatal"
        );
    }

    #[tokio::test]
    async fn live_mode_falls_through_to_the_real_tool_on_divergence() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![recorded("echo", json!({}), "recorded")],
            OnDivergence::Live,
            &cancel,
        );
        let ctx = ToolCtx::default();

        let out = reg
            .get("other")
            .unwrap()
            .call(json!({}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "live: other");
        assert!(!cancel.is_cancelled(), "live mode keeps going");

        // And it stays live: the recorded result for `echo` is never used,
        // because after the divergence it answers a question nobody asked.
        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({"value": "x"}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "live: x");
    }

    #[test]
    fn a_recorded_tool_missing_today_is_an_error_not_a_shrink() {
        let err = replay_registry(
            &["echo".to_string(), "gone".to_string()],
            &live_registry(),
            // No stand-in offered: a missing tool is still fatal, which is
            // what this test has always asserted.
            None,
            Vec::new(),
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .map(|_| ())
        .unwrap_err()
        .to_string();
        assert!(err.contains("gone"), "{err}");
    }

    // ---- the driver, against a scripted model ----

    struct Scripted(Mutex<Vec<CompletionResponse>>);
    #[async_trait]
    impl Provider for Scripted {
        fn id(&self) -> &str {
            "scripted"
        }
        fn default_model(&self) -> &str {
            "scripted-1"
        }
        async fn complete(
            &self,
            _req: &CompletionRequest,
            _sink: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            let mut turns = self.0.lock().unwrap();
            anyhow::ensure!(!turns.is_empty(), "provider ran out of scripted turns");
            Ok(turns.remove(0))
        }
    }

    fn assistant(blocks: Vec<Block>, stop: StopReason) -> CompletionResponse {
        CompletionResponse {
            message: Message::assistant(blocks),
            stop_reason: stop,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Usage::default()
            },
            refusal: None,
            model: "scripted-1".into(),
            malformed_tool_args: 0,
        }
    }

    fn tool_use(id: &str, name: &str, input: Value) -> Block {
        Block::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    async fn drive_scripted(
        turns: Vec<CompletionResponse>,
        calls: Vec<RecordedCall>,
        trajectory: &Trajectory,
    ) -> ReplayReport {
        let cancel = CancellationToken::new();
        let registry = replay_reg(calls, OnDivergence::Stop, &cancel);
        let approver = Arc::new(ModeApprover {
            mode: PermissionMode::Allow,
        });
        let agent = Agent::new(
            Box::new(Scripted(Mutex::new(turns))),
            registry,
            approver.clone(),
            ToolCtx::default(),
            AgentConfig::default(),
            None,
        )
        .unwrap();
        let cx = RunContext::new(ToolCtx::default(), approver)
            .with_cancel(cancel)
            .with_budget(Budget::turns(8));
        drive(&agent, &cx, trajectory).await.unwrap()
    }

    #[tokio::test]
    async fn a_faithful_replay_reports_no_divergence() {
        let calls = vec![recorded("echo", json!({"value": "a"}), "first")];
        let trajectory = Trajectory {
            turns: vec!["do the thing".into()],
            calls: calls.clone(),
            final_text: "done".into(),
            steered: false,
        };
        let report = drive_scripted(
            vec![
                assistant(
                    vec![tool_use("t1", "echo", json!({"value": "a"}))],
                    StopReason::ToolUse,
                ),
                assistant(vec![Block::text("done")], StopReason::EndTurn),
            ],
            calls,
            &trajectory,
        )
        .await;

        assert!(report.divergences.is_empty(), "{:?}", report.divergences);
        assert!(!report.stopped_early);
        assert_eq!(report.final_text, "done");
        assert_eq!(report.turns, 1);
    }

    #[tokio::test]
    async fn a_divergent_replay_stops_early_and_reports_it() {
        let calls = vec![
            recorded("echo", json!({"value": "a"}), "first"),
            recorded("echo", json!({"value": "b"}), "second"),
        ];
        let trajectory = Trajectory {
            turns: vec!["do the thing".into(), "never reached".into()],
            calls: calls.clone(),
            final_text: "done".into(),
            steered: false,
        };
        // The model calls `other` where the recording says `echo`; the refusal
        // comes back as a tool result, and the cancelled run ends the turn.
        let report = drive_scripted(
            vec![
                assistant(
                    vec![tool_use("t1", "other", json!({}))],
                    StopReason::ToolUse,
                ),
                assistant(vec![Block::text("gave up")], StopReason::EndTurn),
            ],
            calls,
            &trajectory,
        )
        .await;

        assert!(
            report.stopped_early,
            "the second recorded turn must never be fed"
        );
        assert_eq!(report.turns, 1);
        let structural: Vec<_> = report.structural().collect();
        assert!(!structural.is_empty(), "{:?}", report.divergences);
    }
}
