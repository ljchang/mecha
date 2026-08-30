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
/// Wraps the live tool rather than replacing it. Under [`OnDivergence::Live`]
/// the spec the model sees — name, description, schema — is the live one,
/// because the live tool is what would actually run after a divergence. Under
/// the non-executing modes, `spec` carries the *recorded* spec when the
/// surface store still holds the blob the recording's `tools_hash` cites, and
/// it wins over the live tool's own words: tools render at the very front of
/// the prompt, and re-describing one is the surface-drift cause
/// [`crate::surface`] was built to name — presenting today's description to a
/// probe of yesterday's recording replays a different question.
/// Capabilities always come from `inner`: the spec says what the model reads,
/// the capability says what the harness must assume, and only the live tool
/// knows the latter.
struct ReplayTool {
    inner: Arc<dyn Tool>,
    /// The recorded spec, when one is available and this mode replays rather
    /// than executes. `None` falls through to `inner`'s own words.
    spec: Option<crate::message::ToolSpec>,
    mode: OnDivergence,
    state: Arc<Mutex<ReplayState>>,
    cancel: CancellationToken,
}

/// A stand-in built from a recorded [`ToolSpec`](crate::message::ToolSpec)
/// alone — for a recorded tool that nothing running today can construct (a
/// retired integration, a renamed MCP prefix). It can describe itself into
/// the request, which is all a non-executing replay ever needs; recorded
/// calls are answered from the recording before this is ever reached, and
/// [`replay_registry`] refuses stand-ins under [`OnDivergence::Live`].
///
/// Capabilities are conservative on the taint axes — unknown is never clean —
/// and `external_send` is narrowed away by the wrapper in the only modes this
/// can exist in, so the interlock stays quiet for the same reason it does
/// over live tools: a replayed call sends nothing.
struct SpecTool {
    spec: crate::message::ToolSpec,
}

#[async_trait]
impl Tool for SpecTool {
    fn name(&self) -> &str {
        &self.spec.name
    }
    fn description(&self) -> &str {
        &self.spec.description
    }
    fn input_schema(&self) -> Value {
        self.spec.input_schema.clone()
    }
    fn read_only(&self) -> bool {
        false
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            private_data: true,
            untrusted_input: true,
            external_send: true,
            destructive: true,
        }
    }
    async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        Ok(ToolOutput::err(
            "this tool exists only on the recorded surface; the replay answers from \
             the recording, and this call has nothing recorded to answer with",
        ))
    }
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
        self.spec
            .as_ref()
            .map(|s| s.name.as_str())
            .unwrap_or_else(|| self.inner.name())
    }
    fn description(&self) -> &str {
        self.spec
            .as_ref()
            .map(|s| s.description.as_str())
            .unwrap_or_else(|| self.inner.description())
    }
    fn input_schema(&self) -> Value {
        self.spec
            .as_ref()
            .map(|s| s.input_schema.clone())
            .unwrap_or_else(|| self.inner.input_schema())
    }
    fn read_only(&self) -> bool {
        self.inner.read_only()
    }
    /// The live tool's capabilities, minus `external_send` in the modes where
    /// nothing executes — the sandbox's rule in the other direction. A
    /// replayed "send" sends nothing: its answer comes from the recording, so
    /// declaring the capability would let the loop's trifecta interlock fire
    /// *inside the replay* and block a call the recording executed. A blocked
    /// call never reaches this tool, the cursor never consumes its recorded
    /// result, and the arm dies one call later for a harness reason — which
    /// grades as the model's failure. Measured 2026-08-29: `docs__sheets_write`
    /// and `web_search` blocked mid-probe on three of twelve arms. Narrowing
    /// also repairs record-time blocks: the interlock's refusal *is* the
    /// recorded result, so the call now consumes it and the model reads back
    /// exactly the refusal it read then. `private_data` stays, as everywhere —
    /// and under `Live`, where the tool genuinely runs, nothing narrows.
    fn capabilities(&self) -> Capabilities {
        let caps = self.inner.capabilities();
        match self.mode {
            OnDivergence::Live => caps,
            OnDivergence::Stop | OnDivergence::Error => Capabilities {
                external_send: false,
                ..caps
            },
        }
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        // Decided under the lock, executed after it: a live call awaits, and a
        // std mutex must not be held across an await point.
        match self.decide(&input) {
            Action::Recorded(content, is_error) => Ok(ToolOutput {
                content,
                is_error,
                external: false,
                refusal: false,
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
    recorded_specs: &[crate::message::ToolSpec],
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
        // An exhaustive match rather than `matches!(mode, OnDivergence::Live)`:
        // a `matches!` boolean stays green when a mode is added and nobody
        // decided whether it executes, which is exactly the shape this gate's
        // own history says took two wrong answers to get right. Naming every
        // arm makes a new variant a compile error here until someone chooses.
        let executes = match mode {
            OnDivergence::Live => true,
            OnDivergence::Stop | OnDivergence::Error => false,
        };
        // The recorded spec, when the surface store still holds the blob the
        // recording cites. In the non-executing modes it does two jobs: it
        // overrides a live tool's *words* (re-describe is the drift that
        // happens constantly, and tools render at the very front of the
        // prompt), and it stands in bodily for a tool nothing today can
        // construct — a retired integration, a renamed MCP prefix — which
        // used to skip the probe outright. Under `Live` it does neither: the
        // tools genuinely run there, and they deserve their own words.
        let spec = (!executes)
            .then(|| recorded_specs.iter().find(|s| s.name == *name))
            .flatten();
        let stand_in = (!executes)
            .then(|| surface_only.and_then(|r| r.get(name)))
            .flatten();
        let inner: Arc<dyn Tool> = match live.get(name).or(stand_in) {
            Some(tool) => Arc::clone(tool),
            None => match spec {
                Some(s) => Arc::new(SpecTool { spec: s.clone() }),
                None => bail!(
                    "recorded tool `{name}` is not available now, so the replay cannot \
                     offer the tool surface the model saw. Enable whatever provided it \
                     (an MCP server? a search backend?) and retry — or, if it no longer \
                     exists anywhere, only a recording whose surface blob the store \
                     still holds (`tools_hash`) can be replayed without it"
                ),
            },
        };
        registry.insert(Arc::new(ReplayTool {
            inner,
            spec: spec.cloned(),
            mode,
            state: Arc::clone(&state),
            cancel: cancel.clone(),
        }));
    }
    Ok(registry)
}

/// The specs of the surface a replay of `recorded_tools` would actually send.
///
/// A fidelity check exists to answer "does the replay send the bytes the
/// recording sent", so it must fingerprint the registry a replay *builds* —
/// the recorded names, narrowed from `live` and filled from `surface_only`
/// for names `live` cannot construct (`ask_user`, `recall`, `show_file`,
/// registered only by a front-end that owns a human, and present on the
/// recorded surface of every interactive session that ever contains a
/// steer). Comparing against the bare `live` registry's own specs instead —
/// the earlier mistake — reports `Differs` on nearly every probe, because no
/// CLI process can ever hold those names, recorded surface or not.
///
/// This calls [`replay_registry`] itself rather than re-deriving which tool
/// wins between `live` and `surface_only`, so the specs it returns are
/// provably the ones a replay would send and cannot drift from that policy.
/// Nothing here executes a tool — no call is ever dispatched through the
/// registry this builds — so an empty call list and a throwaway
/// cancellation token are enough.
pub fn replay_surface_specs(
    recorded_tools: &[String],
    live: &Registry,
    surface_only: Option<&Registry>,
    recorded_specs: &[crate::message::ToolSpec],
) -> Result<Vec<crate::message::ToolSpec>> {
    let registry = replay_registry(
        recorded_tools,
        live,
        surface_only,
        recorded_specs,
        Vec::new(),
        OnDivergence::Stop,
        CancellationToken::new(),
    )?;
    Ok(registry.specs())
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
    /// Global index of the first call the model itself produced. Zero for a
    /// full replay. A branched replay ([`drive_branch`]) resubmits the
    /// recording up to an intervention verbatim, so its forced prefix cannot
    /// diverge and every index in `divergences` is at or above this —
    /// `replayed_calls[i]` sits at recording position `call_base + i`.
    pub call_base: usize,
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
        call_base: 0,
    })
}

/// Continue a recorded conversation from a branch point instead of
/// regenerating it.
///
/// [`drive`] asks the model to re-decide every call from the first user turn,
/// which makes reaching a *mid-run* probe point a lottery: every open choice
/// before the point is a chance to fork, so the odds decay with the point's
/// depth — measured on the live store, 11 of 12 steer probes were lost to
/// pre-point divergence, most at call #1 against points at #10–#28. This
/// driver removes the lottery structurally. `seed` — the recorded messages up
/// to the intervention, steering text already removed — is resubmitted
/// verbatim, which is exactly the wire shape an ongoing run has at that
/// moment, so the model's first free choice *is* the one the probe asks
/// about, and a divergence before the branch cannot exist. The forced prefix
/// also matches the recording's byte prefix, so on a caching server it reads
/// from KV cache instead of being regenerated token by token.
///
/// `call_base` is the recording's index of the first regenerated call. The
/// agent's [`replay_registry`] must have been built over the matching tail —
/// `trajectory.calls[call_base..]` — or every recorded answer arrives offset.
/// Divergences come back in the full recording's coordinates
/// ([`crate::replay::diff_from`]), which is where a probe point lives.
///
/// One `run_in`, no turn loop: a branch point sits inside a run, and
/// [`crate::counterfactual::truncate_after_run`] has already cut the slice
/// before the next top-level turn, so there is nothing left to feed after
/// the continuation ends.
pub async fn drive_branch(
    agent: &Agent,
    cx: &RunContext,
    seed: Vec<Message>,
    trajectory: &Trajectory,
    call_base: usize,
) -> Result<ReplayReport> {
    let base = call_base.min(trajectory.calls.len());
    // A fresh default taint, as `drive` starts with: replayed results carry no
    // `external` marking, so the branch may be less armed than the recording
    // was — the module doc's standing caveat, unchanged here.
    let mut convo = Conversation::resumed(seed, Default::default());
    let mut stats = crate::session::RunStats {
        usage_complete: true,
        ..Default::default()
    };
    let outcome = agent.run_in(cx, &mut convo, None).await?;
    stats.absorb(&outcome);
    Ok(ReplayReport {
        divergences: crate::replay::diff_from(base, &trajectory.calls[base..], &outcome.tool_calls),
        replayed_calls: outcome.tool_calls,
        recorded_calls: trajectory.calls.len(),
        // No user turns are fed on a branch — the seed carries them all.
        turns: 0,
        stopped_early: cx.cancelled(),
        final_text: outcome.text,
        stats,
        call_base: base,
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

    /// A tool that fails the test if it is ever executed. Stands in for
    /// `ask_user`, whose execution would put a question in front of a human.
    struct NeverRun;
    #[async_trait]
    impl Tool for NeverRun {
        fn name(&self) -> &str {
            "never_run"
        }
        fn description(&self) -> &str {
            "Described, never called."
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn read_only(&self) -> bool {
            true
        }
        async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
            panic!("a surface-only tool was executed by a replay");
        }
    }

    /// A stand-in is **described and never run**, which is the whole safety
    /// argument for letting one fill a gap at all.
    ///
    /// The tool this exists for is `ask_user`: putting it into an unattended
    /// replay would be indefensible if the replay could call it, because the
    /// call blocks on a person. It cannot — a recorded call is answered from
    /// the recording, and `NeverRun` panics if that is ever untrue. Asserting
    /// it here rather than reasoning about `Action::Live` being unreachable,
    /// because the reasoning is what would rot if the dispatch changed.
    #[tokio::test]
    async fn a_surface_only_tool_is_described_and_never_executed() {
        let mut fallback = Registry::new();
        fallback.insert(Arc::new(NeverRun));
        let registry = replay_registry(
            &[NeverRun.name().to_string()],
            &Registry::new(),
            Some(&fallback),
            &[],
            vec![RecordedCall {
                name: NeverRun.name().to_string(),
                input: json!({}),
                output: "the recorded answer".into(),
                is_error: false,
            }],
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .expect("the stand-in supplies the surface");

        let tool = registry.get(NeverRun.name()).expect("registered");
        // Described with the real tool's own words, so the model sees what the
        // recording showed it rather than a paraphrase.
        assert_eq!(tool.description(), NeverRun.description());
        assert_eq!(tool.input_schema(), NeverRun.input_schema());

        // And answered from the recording. `NeverRun::call` panics.
        let out = tool
            .call(json!({}), &ToolCtx::default())
            .await
            .expect("answered from the recording");
        assert_eq!(out.content, "the recorded answer");
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
            &[],
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
            &[],
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
            &[],
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
            &[],
            Vec::new(),
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .is_err());
    }

    /// The regression this module exists to prevent: fingerprinting the bare
    /// `live` registry instead of the surface a replay actually builds.
    ///
    /// `other` here stands in for `ask_user` — a name no CLI registry can
    /// ever construct, supplied only by `surface_only`, exactly as it is on
    /// the recorded surface of every interactive session that ever contains
    /// a steer. The recording was made by a process that *did* hold it (a
    /// TUI, say), so its hash covers both tools; comparing that hash against
    /// `live`'s own specs — the old, wrong comparand — can never match,
    /// because `live` never has `other` at all. Comparing it against
    /// [`replay_surface_specs`]'s output does, because that is the surface
    /// the replay actually sends. No existing fingerprint test can catch
    /// this: they all compare two values computed from the same registry, and
    /// this bug only shows up when the recorded and live registries
    /// genuinely differ in what they can construct.
    #[test]
    fn fidelity_compares_the_replay_surface_not_the_bare_live_registry() {
        let mut live = Registry::new();
        live.insert(Arc::new(EchoTool));

        let mut surface_only = Registry::new();
        surface_only.insert(Arc::new(OtherTool));

        let recorded_tools = vec![EchoTool.name().to_string(), OtherTool.name().to_string()];

        // What was recorded: a process holding both tools, so the hash covers
        // the full surface the model actually saw.
        let mut recorded_registry = Registry::new();
        recorded_registry.insert(Arc::new(EchoTool));
        recorded_registry.insert(Arc::new(OtherTool));
        let recorded_hash = crate::surface::fingerprint(&recorded_registry.specs());

        // The wrong comparand: `live` alone can never hold `other`, so this
        // reports `Differs` regardless of whether anything really changed.
        assert_eq!(
            crate::surface::Fidelity::of(Some(&recorded_hash), &live.specs()),
            crate::surface::Fidelity::Differs,
            "live alone lacks `other` by construction — this is the bug being fixed"
        );

        // The fix: fingerprint the surface a replay actually builds, narrowed
        // to the recorded names and filled from the surface-only stand-in.
        let surface_specs =
            replay_surface_specs(&recorded_tools, &live, Some(&surface_only), &[]).unwrap();
        assert_eq!(
            crate::surface::Fidelity::of(Some(&recorded_hash), &surface_specs),
            crate::surface::Fidelity::Matches,
            "identical specs, narrowed the same way the recording was — this must match"
        );
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
            &[],
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

    /// A tool that can send when live sends nothing when replayed — its
    /// answers come from the recording — so under the non-executing modes the
    /// wrapper must not declare `external_send`, or the loop's trifecta
    /// interlock blocks mid-replay a call the recording executed, the cursor
    /// never consumes that call's recorded result, and the arm dies one call
    /// later for a harness reason graded as the model's. Everything else
    /// stays: `private_data` narrowing would under-taint the replay.
    #[tokio::test]
    async fn replay_narrows_external_send_in_the_modes_where_nothing_executes() {
        struct Sender;
        #[async_trait]
        impl Tool for Sender {
            fn name(&self) -> &str {
                "mail_send"
            }
            fn description(&self) -> &str {
                "Sends."
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object"})
            }
            fn read_only(&self) -> bool {
                false
            }
            fn capabilities(&self) -> Capabilities {
                Capabilities {
                    private_data: true,
                    external_send: true,
                    ..Default::default()
                }
            }
            async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
                Ok(ToolOutput::ok("sent"))
            }
        }

        let mut live = Registry::new();
        live.insert(Arc::new(Sender));
        let caps_under = |mode| {
            let reg = replay_registry(
                &["mail_send".to_string()],
                &live,
                None,
                &[],
                Vec::new(),
                mode,
                CancellationToken::new(),
            )
            .unwrap();
            reg.get("mail_send").unwrap().capabilities()
        };

        for mode in [OnDivergence::Stop, OnDivergence::Error] {
            let caps = caps_under(mode);
            assert!(!caps.external_send, "a replayed send sends nothing");
            assert!(caps.private_data, "private data must not narrow");
        }
        // Under Live the tool genuinely runs, and the interlock deserves the
        // truth about it.
        assert!(caps_under(OnDivergence::Live).external_send);
    }

    /// The skip this closes: a recorded tool nothing today can construct — a
    /// retired integration, a renamed MCP prefix — used to bail the whole
    /// probe, four reflections' worth on the live store. When the surface
    /// store still holds the recording's spec blob, the spec alone rebuilds
    /// the surface under the non-executing modes; recorded calls are answered
    /// from the recording, so the stand-in's own `call` never runs on a
    /// faithful path. Under `Live`, where tools genuinely execute, a spec
    /// that cannot execute is still no substitute — fatal, unchanged.
    #[tokio::test]
    async fn a_dead_recorded_tool_is_rebuilt_from_its_recorded_spec_under_stop() {
        let recorded_spec = crate::message::ToolSpec {
            name: "pkg__kg_entity".into(),
            description: "Look up one entity in the knowledge graph.".into(),
            input_schema: json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        };
        let specs = vec![recorded_spec.clone()];
        let names = vec![recorded_spec.name.clone()];

        let registry = replay_registry(
            &names,
            &Registry::new(),
            None,
            &specs,
            vec![RecordedCall {
                name: recorded_spec.name.clone(),
                input: json!({"name": "Yuqi"}),
                output: "the recorded entity".into(),
                is_error: false,
            }],
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .expect("the recorded spec supplies the surface");

        let tool = registry.get(&recorded_spec.name).expect("registered");
        // Described with the recording's own words — the bytes the model saw.
        assert_eq!(tool.description(), recorded_spec.description);
        assert_eq!(tool.input_schema(), recorded_spec.input_schema);
        // Conservative on the taint axes, narrowed on the send axis.
        assert!(tool.capabilities().private_data, "unknown is never clean");
        assert!(
            !tool.capabilities().external_send,
            "a replayed call sends nothing"
        );
        // And answered from the recording, never executed.
        let out = tool
            .call(json!({"name": "Yuqi"}), &ToolCtx::default())
            .await
            .unwrap();
        assert_eq!(out.content, "the recorded entity");

        // Live mode executes, and a spec cannot: still fatal.
        assert!(replay_registry(
            &names,
            &Registry::new(),
            None,
            &specs,
            Vec::new(),
            OnDivergence::Live,
            CancellationToken::new(),
        )
        .is_err());
    }

    /// Re-describe is the drift that happens constantly, and tools render at
    /// the very front of the prompt — so when the store holds the recorded
    /// spec, it must win over the live tool's words under the non-executing
    /// modes (the replay sends the bytes the recording sent) and lose under
    /// `Live` (the live tool is what would actually run). Fidelity follows:
    /// the rebuilt surface fingerprints back to the recorded hash even after
    /// every live description has moved on.
    #[test]
    fn a_recorded_spec_overrides_a_live_tools_words_under_stop_only() {
        let recorded_spec = crate::message::ToolSpec {
            name: EchoTool.name().into(),
            description: "The words the recording was sent.".into(),
            input_schema: json!({"type": "object"}),
        };
        let specs = vec![recorded_spec.clone()];
        let names = vec![recorded_spec.name.clone()];
        let recorded_hash = crate::surface::fingerprint(&specs);

        let stopped = replay_registry(
            &names,
            &live_registry(),
            None,
            &specs,
            Vec::new(),
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(
            stopped.get("echo").unwrap().description(),
            recorded_spec.description,
            "a probe replays the recorded surface, not today's rewording"
        );

        let surface = replay_surface_specs(&names, &live_registry(), None, &specs).unwrap();
        assert_eq!(
            crate::surface::Fidelity::of(Some(&recorded_hash), &surface),
            crate::surface::Fidelity::Matches,
            "the rebuilt surface is byte-faithful to the recording"
        );

        let live = replay_registry(
            &names,
            &live_registry(),
            None,
            &specs,
            Vec::new(),
            OnDivergence::Live,
            CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(
            live.get("echo").unwrap().description(),
            EchoTool.description(),
            "live mode executes today's tool and must show today's words"
        );
    }

    #[test]
    fn a_recorded_tool_missing_today_is_an_error_not_a_shrink() {
        let err = replay_registry(
            &["echo".to_string(), "gone".to_string()],
            &live_registry(),
            // No stand-in offered: a missing tool is still fatal, which is
            // what this test has always asserted.
            None,
            &[],
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

    /// The branch driver: the recorded prefix is resubmitted, not
    /// regenerated, so the model's first sampled choice is the first call
    /// after the branch point — and a faithful continuation reports nothing,
    /// while a fork reports its divergence in the *recording's* coordinates.
    #[tokio::test]
    async fn a_branch_continues_the_recording_instead_of_regenerating_it() {
        let calls = vec![
            recorded("echo", json!({"value": "a"}), "first"),
            recorded("other", json!({}), "second"),
        ];
        let trajectory = Trajectory {
            turns: vec!["do the thing".into()],
            calls: calls.clone(),
            final_text: "done".into(),
            steered: true,
        };
        // The forced prefix: the first call already made and answered.
        let seed = vec![
            Message::user("do the thing"),
            Message::assistant(vec![tool_use("t1", "echo", json!({"value": "a"}))]),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "first".into(),
                is_error: false,
            }]),
        ];

        let run = |turns: Vec<CompletionResponse>| {
            let calls = calls.clone();
            let trajectory = trajectory.clone();
            let seed = seed.clone();
            async move {
                let cancel = CancellationToken::new();
                // The registry answers from the tail — the branch's contract.
                let registry = replay_reg(calls[1..].to_vec(), OnDivergence::Stop, &cancel);
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
                drive_branch(&agent, &cx, seed, &trajectory, 1)
                    .await
                    .unwrap()
            }
        };

        // Faithful: the continuation makes the one remaining recorded call.
        let report = run(vec![
            assistant(
                vec![tool_use("t2", "other", json!({}))],
                StopReason::ToolUse,
            ),
            assistant(vec![Block::text("done")], StopReason::EndTurn),
        ])
        .await;
        assert!(report.divergences.is_empty(), "{:?}", report.divergences);
        assert_eq!(report.call_base, 1);
        assert_eq!(
            report.replayed_calls.len(),
            1,
            "only the continuation is the model's"
        );
        assert_eq!(report.final_text, "done");

        // Forked: the divergence is reported at recording position 1, not at
        // the continuation's local position 0 — the coordinates a probe
        // point is located in.
        let report = run(vec![
            assistant(
                vec![tool_use("t2", "echo", json!({"value": "z"}))],
                StopReason::ToolUse,
            ),
            assistant(vec![Block::text("gave up")], StopReason::EndTurn),
        ])
        .await;
        let structural: Vec<_> = report.structural().collect();
        assert!(!structural.is_empty(), "{:?}", report.divergences);
        assert_eq!(structural[0].index(), 1, "{:?}", report.divergences);
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
