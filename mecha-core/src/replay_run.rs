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
//! external data, but does not guarantee an identical model-facing transcript:
//! the loop reapplies its output limits and untrusted-content warning envelope.
//!
//! New recordings preserve per-call provenance, including explicit false for
//! harness refusals. Legacy results have unknown provenance and conservatively
//! count as external, even if the original result was harness-owned. They may
//! gain a warning, or a second envelope when the recording already had one.
//! Under live divergence the stronger taint can also block a send the original
//! run allowed. A picture a tool showed the model (`ToolOutput::image`) is not
//! replayed: it rode in the results turn, and a recorded answer is text. These
//! are harness differences, not evidence of model regression;
//! `mecha replay` discloses them in its note and JSON report. Both comparison
//! arms must replay under the same policy. Never mark unknown results clean to
//! improve fidelity: past a divergence, live mode executes the unguarded
//! pure reads its [`LiveGate`] admits, and stops in front of anything else.

use crate::agent::{Agent, Conversation, RunContext, ToolCallTrace};
use crate::message::Message;
use crate::replay::{diff, Divergence, RecordedCall, Trajectory};
use crate::tool::{Capabilities, Egress, Registry, Tool, ToolCtx, ToolOutput};
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
    /// Leave the recording and keep going against the live tools — but only
    /// as far as a [`LiveGate`] allows: unguarded pure reads run, and the run
    /// stops in front of the first call anything configured would intercept,
    /// recording it ([`DivergenceProbe::halted`]). Built only through
    /// [`replay_registry_live`]. The comparison is over at the divergence.
    Live,
}

impl OnDivergence {
    /// Whether tools genuinely run under this mode, which decides whether a
    /// recorded spec or a surface-only stand-in may substitute for one.
    ///
    /// An exhaustive match rather than `matches!(self, Self::Live)`: a
    /// `matches!` boolean stays green when a mode is added and nobody decided
    /// whether it executes, which is exactly the shape this gate's own history
    /// says took two wrong answers to get right. Naming every arm makes a new
    /// variant a compile error here until someone chooses.
    pub fn executes(self) -> bool {
        match self {
            Self::Live => true,
            Self::Stop | Self::Error => false,
        }
    }
}

/// The shared cursor over the recording. One per replay, shared by every
/// [`ReplayTool`] in the registry, because the recording is one sequence — the
/// order calls happen in *is* the trajectory, whichever tool they went to.
struct ReplayState {
    calls: Vec<RecordedCall>,
    /// The first recorded call not yet answered. Only ever moves forward, and
    /// only over calls `consumed` marks done.
    cursor: usize,
    /// Which recorded calls have been answered.
    ///
    /// A flag rather than removing them from `calls`: the divergence messages
    /// and `call_base` both count in the recording's own coordinates, and
    /// shifting the vector out from under those is how an offset report gets
    /// written. Needed at all because a batch may be answered out of order.
    consumed: Vec<bool>,
    /// Set at the first structural divergence. From then on the recording has
    /// nothing truthful left to say.
    dead: bool,
    /// Why the recording was left, set once beside `dead`.
    ///
    /// The message already existed — it is what the diverging call is
    /// refused with — but it went only to the model, so every caller
    /// upstream saw a bare bool. `mecha harness show` could name the
    /// episodes that diverged and nothing about how, which is the
    /// difference between "the replay is unreliable" and "the change did
    /// something", and those want opposite responses.
    divergence: Option<String>,
    /// What may still run once the recording is left. Read only under
    /// [`OnDivergence::Live`]; `None` there refuses every live call, so a
    /// live replay built without one executes nothing past the divergence.
    gate: Option<Arc<LiveGate>>,
    /// Every call a live replay stopped in front of, in the order they were
    /// refused. A vector rather than one slot so that no refusal that reaches
    /// the gate is dropped; whether a batch sibling still reaches it after the
    /// first refusal cancels the run depends on where dispatch reads
    /// cancellation, so a batch may report one refusal or more.
    halted: Vec<Halted>,
}

/// What a live replay may still execute once it has left the recording:
/// **pure reads that nothing the operator configured guards** — and at the
/// first call that is anything else, it stops in front of it
/// (`docs/ARCHITECTURE.md` §Session records and replay).
///
/// The guards themselves cannot ride on the run: staging, an approval rule
/// and a `pre_tool` hook all act in the loop *ahead of* the tool, so a
/// recorded call they intercept before the divergence never reaches
/// [`ReplayTool`], is never consumed, and the replay dies one call later for
/// a harness reason graded as the model's (the shape
/// [`ReplayTool::capabilities`] documents for the interlock). So they are
/// asked here instead, and only of calls that would genuinely run.
///
/// Stopping rather than staging is the point, not a shortcut: the first
/// consequential call is where a branch's direction is already visible, and
/// the call and its arguments are the result.
pub struct LiveGate {
    routed: std::collections::BTreeSet<String>,
    policy: Arc<crate::policy::ExecPolicy>,
    hooks: Arc<crate::hooks::HookSet>,
}

/// Built-in tools that write while answering `read_only()`, which skips the
/// approver in a real run. Named here until the effect class of
/// branch-continuation design replaces the list (`docs/ARCHITECTURE.md`
/// §Session records and replay).
pub const READ_ONLY_WRITERS: &[&str] = &[
    "ask_user",
    "http_fetch",
    "hud_propose",
    "image_generate",
    "image_library_propose",
    "image_library_reference",
    "message_send",
    "persona_propose",
    // Posts a file into a Slack thread. Defined in mecha-cli: a name, not a
    // dependency, and an accepted cost of the list being temporary. The
    // effect class that replaces it is declared where each tool lives.
    "show_file",
];

impl LiveGate {
    /// `routed` is the operator's `[outbox] tools` and `publish_tools`; the
    /// policy and hooks are the ones `setup` built for the run.
    pub fn new(
        routed: impl IntoIterator<Item = String>,
        policy: Arc<crate::policy::ExecPolicy>,
        hooks: Arc<crate::hooks::HookSet>,
    ) -> Self {
        LiveGate {
            routed: routed.into_iter().collect(),
            policy,
            hooks,
        }
    }

    /// Pure reads through, nothing configured: the gate with no operator.
    /// Test-only: a production gate is always built from a run's configured
    /// route, rules and hooks (`LiveGate::new`), and a public operator-free
    /// gate would satisfy `replay_registry_live` while ignoring all three.
    #[cfg(test)]
    pub(crate) fn reads_only() -> Self {
        Self::new(
            Vec::new(),
            Arc::new(crate::policy::ExecPolicy::empty()),
            Arc::new(crate::hooks::HookSet::default()),
        )
    }

    /// Why `tool` may not run past the divergence, or `None` when it is a
    /// pure read nothing configured guards.
    ///
    /// Trusts `read_only()` and the declared capabilities exactly as far as
    /// a real run does — the approver skips a read-only tool there too — so
    /// a live replay past this gate is never less guarded than the run it
    /// replays, and stops before anything that run would have staged, asked
    /// about, or written.
    pub fn refuses(&self, tool: &dyn Tool, input: &Value) -> Option<String> {
        let name = tool.name();
        if self.routed.contains(name) {
            return Some("the outbox routes it".into());
        }
        if READ_ONLY_WRITERS.contains(&name) {
            return Some("it writes, although it reports itself read-only".into());
        }
        // A subagent reports `read_only()` and derives no `destructive` from
        // its child, so its legs below could read as a pure read while the
        // child it starts writes live. Delegating must not be the way around
        // this gate, any more than around the parent's hooks or outbox.
        if tool.runs_a_fresh_conversation() {
            return Some("it starts a child run whose tools this gate cannot see".into());
        }
        let caps = tool.capabilities();
        if !tool.read_only() || caps.destructive {
            return Some("it writes".into());
        }
        if caps.egress != Egress::None {
            return Some("it can send to a destination".into());
        }
        if let Some(rule) = self.policy.decide(name, input) {
            if rule.decision != crate::policy::RuleDecision::Allow {
                return Some(format!("an approval rule covers it: {}", rule.reason));
            }
        }
        if self.hooks.guards_tool(name) {
            return Some("a pre_tool hook watches it".into());
        }
        None
    }
}

/// The call a live replay stopped in front of, and why.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Halted {
    pub tool: String,
    pub input: Value,
    pub reason: String,
}

/// What one call decided to do, resolved under the lock and acted on after it.
enum Action {
    Recorded(String, bool, Option<bool>),
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
            egress: Egress::Chosen,
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

        while st.cursor < st.calls.len() && st.consumed[st.cursor] {
            st.cursor += 1;
        }

        if st.cursor >= st.calls.len() {
            // The model kept going past the end of the recording.
            let why = format!(
                "the recording ended after {} call(s) and has no result for this one",
                st.calls.len()
            );
            st.dead = true;
            st.divergence.get_or_insert_with(|| why.clone());
            return match self.mode {
                OnDivergence::Live => Action::Live,
                _ => {
                    self.cancel.cancel();
                    Action::Refuse(format!("replay: {why}; stopping"))
                }
            };
        }

        // Anywhere in the current batch, not just at the cursor. The calls in
        // one assistant turn are issued together and run concurrently, so the
        // order the model emitted them in is not a decision — insisting on it
        // killed four of the twelve episodes the 2026-09-08 harness pass
        // dropped, each on a batch it had reproduced exactly. An unmarked call
        // is its own batch, which is the old strict behaviour — nothing in
        // production is unmarked, see `RecordedCall::batch` — and
        // `replay::diff` groups by the same marker so the report agrees with
        // what this accepted.
        let batch = st.calls[st.cursor].batch;
        let name = self.inner.name();
        let hit = match batch {
            None => (st.calls[st.cursor].name == name).then_some(st.cursor),
            Some(_) => {
                let end = st.cursor
                    + st.calls[st.cursor..]
                        .iter()
                        .take_while(|c| c.batch == batch)
                        .count();
                // Arguments first, then name alone — the two passes
                // `replay::pair_batch` makes, and for the same reason. A batch
                // that called one tool twice has no order to say which call is
                // which, so pairing by name would hand each call its sibling's
                // output. The diff pairs by arguments and would then report the
                // episode clean over it, which is worse than the strict cursor
                // this replaced: that fed the same wrong outputs but at least
                // reported two argument divergences. `decide` and `pair_batch`
                // must choose the same mate.
                (st.cursor..end)
                    .find(|&j| {
                        !st.consumed[j]
                            && st.calls[j].name == name
                            && crate::replay::same_arguments(&st.calls[j].input, input)
                    })
                    .or_else(|| {
                        (st.cursor..end).find(|&j| !st.consumed[j] && st.calls[j].name == name)
                    })
            }
        };

        let Some(hit) = hit else {
            let why = format!(
                "recorded call #{} was `{}`, not `{}`",
                st.cursor, st.calls[st.cursor].name, name
            );
            let msg = format!("replay: {why}; stopping");
            st.dead = true;
            st.divergence.get_or_insert(why);
            return match self.mode {
                OnDivergence::Live => Action::Live,
                _ => {
                    self.cancel.cancel();
                    Action::Refuse(msg)
                }
            };
        };

        // Same tool. Different arguments are *not* grounds to stop: a path
        // spelled differently is the same decision, and the final diff reports
        // every argument difference for the caller to judge. Returning the
        // recorded result for materially different arguments is the price of
        // not pretending to know which differences matter — which is what the
        // name-only fallback above is, once the exact match has been tried.
        let out = Action::Recorded(
            st.calls[hit].output.clone(),
            st.calls[hit].is_error,
            st.calls[hit].external,
        );
        st.consumed[hit] = true;
        while st.cursor < st.calls.len() && st.consumed[st.cursor] {
            st.cursor += 1;
        }
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

    /// Delegated with the rest of the family: `LiveGate` refuses a tool that
    /// starts a child run, and boredom looks for one. The wrapper answering
    /// `false` for a subagent would be the silently wrong default.
    fn runs_a_fresh_conversation(&self) -> bool {
        self.inner.runs_a_fresh_conversation()
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
    /// Delegated with the rest of what only the live tool knows (review of
    /// #666, pass 5): `false` would be the silently wrong default.
    fn defers(&self) -> bool {
        self.inner.defers()
    }

    /// Delegated whatever the mode, and the reason is `defers`' own: the
    /// default `None` would be the silently wrong answer. Under `Live` the
    /// tool genuinely runs, and a call that is a person's decision
    /// (`request_access`) must still reach one — a replay is not a person.
    fn needs_a_person(&self, input: &Value, ctx: &ToolCtx) -> Option<crate::tool::PersonAsk> {
        self.inner.needs_a_person(input, ctx)
    }

    fn capabilities(&self) -> Capabilities {
        let caps = self.inner.capabilities();
        match self.mode {
            OnDivergence::Live => caps,
            OnDivergence::Stop | OnDivergence::Error => Capabilities {
                egress: Egress::None,
                ..caps
            },
        }
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        // Decided under the lock, executed after it: a live call awaits, and a
        // std mutex must not be held across an await point.
        match self.decide(&input) {
            Action::Recorded(content, is_error, external) => Ok(ToolOutput {
                deferred: None,
                content,
                is_error,
                external: external.unwrap_or(true),
                refusal: false,
                not_dispatched: false,
                // The recorded pixels are not re-sent; `mecha replay` counts
                // and discloses them (`tool_pictures_without_pixels`).
                image: None,
            }),
            Action::Refuse(msg) => Ok(ToolOutput::err(msg)),
            Action::Live => {
                let refused = {
                    let state = self.state.lock().unwrap();
                    match &state.gate {
                        Some(gate) => gate.refuses(self.inner.as_ref(), &input),
                        None => Some("no live gate was given".into()),
                    }
                };
                let Some(reason) = refused else {
                    return self.inner.call(input, ctx).await;
                };
                let tool = self.inner.name().to_string();
                let msg = format!(
                    "replay: stopped before `{tool}`, a consequential call past the \
                     divergence ({reason}); this call was not executed"
                );
                self.state.lock().unwrap().halted.push(Halted {
                    tool,
                    input,
                    reason,
                });
                self.cancel.cancel();
                Ok(ToolOutput::err(msg))
            }
        }
    }
}

/// A read-only handle on why a replay left its recording.
///
/// Returned beside the registry by [`replay_registry_reporting`]. Held
/// separately rather than folded into the report, because the state it reads
/// is shared by every tool in the registry and outlives any one call.
#[derive(Clone)]
pub struct DivergenceProbe(Arc<Mutex<ReplayState>>);

impl DivergenceProbe {
    /// Why this replay diverged, or `None` if it never did.
    pub fn reason(&self) -> Option<String> {
        self.0.lock().unwrap().divergence.clone()
    }

    /// Every call a live replay stopped in front of, empty if it never
    /// stopped.
    pub fn halted(&self) -> Vec<Halted> {
        self.0.lock().unwrap().halted.clone()
    }
}

/// [`replay_registry_reporting`] under [`OnDivergence::Live`], with the
/// [`LiveGate`] that decides what may run once the recording is left.
///
/// The only way to give a live replay a gate. One built through the other
/// constructors has none, and so executes nothing past its divergence —
/// fail closed, the way a configured guard that cannot run must.
pub fn replay_registry_live(
    recorded_tools: &[String],
    live: &Registry,
    recorded_specs: &[crate::message::ToolSpec],
    calls: Vec<RecordedCall>,
    gate: LiveGate,
    cancel: CancellationToken,
) -> Result<(Registry, DivergenceProbe)> {
    // No surface-only stand-ins: under `Live` a recorded tool nothing can
    // construct stays fatal (`build_replay_registry`).
    let (registry, probe) = registry_with_state(
        recorded_tools,
        live,
        None,
        recorded_specs,
        calls,
        OnDivergence::Live,
        cancel,
    )?;
    probe.0.lock().unwrap().gate = Some(Arc::new(gate));
    Ok((registry, probe))
}

/// [`replay_registry`], plus a handle on the divergence reason. Refuses
/// [`OnDivergence::Live`]: a live replay is built only by
/// [`replay_registry_live`], which installs its gate. See
/// [`build_replay_registry`] for the registry contract itself.
///
/// A sibling rather than a changed signature: fifteen call sites want only
/// the registry, and widening all of them to discard a value would be a
/// larger diff than the finding is worth.
pub fn replay_registry_reporting(
    recorded_tools: &[String],
    live: &Registry,
    surface_only: Option<&Registry>,
    recorded_specs: &[crate::message::ToolSpec],
    calls: Vec<RecordedCall>,
    mode: OnDivergence,
    cancel: CancellationToken,
) -> Result<(Registry, DivergenceProbe)> {
    // A live replay without its gate is a programming error, not a run
    // outcome: refused here, where the caller is, rather than reported later
    // as though the branch had stopped at a consequential call.
    anyhow::ensure!(
        !mode.executes(),
        "a live replay needs a gate: build it with `replay_registry_live`"
    );
    registry_with_state(
        recorded_tools,
        live,
        surface_only,
        recorded_specs,
        calls,
        mode,
        cancel,
    )
}

fn registry_with_state(
    recorded_tools: &[String],
    live: &Registry,
    surface_only: Option<&Registry>,
    recorded_specs: &[crate::message::ToolSpec],
    calls: Vec<RecordedCall>,
    mode: OnDivergence,
    cancel: CancellationToken,
) -> Result<(Registry, DivergenceProbe)> {
    let state = Arc::new(Mutex::new(ReplayState {
        consumed: vec![false; calls.len()],
        calls,
        cursor: 0,
        dead: false,
        divergence: None,
        gate: None,
        halted: Vec::new(),
    }));
    let probe = DivergenceProbe(Arc::clone(&state));
    let registry = build_replay_registry(
        recorded_tools,
        live,
        surface_only,
        recorded_specs,
        state,
        mode,
        cancel,
    )?;
    Ok((registry, probe))
}

/// Build a registry that offers the recorded tool surface and answers from
/// the recording. See [`build_replay_registry`] for the full contract —
/// which names must resolve, and how `surface_only` is gated on the mode.
///
/// Use [`replay_registry_reporting`] instead where the caller needs to know
/// *why* a replay diverged rather than only that it did.
/// Refuses [`OnDivergence::Live`], through [`replay_registry_reporting`]:
/// build a live replay with [`replay_registry_live`].
pub fn replay_registry(
    recorded_tools: &[String],
    live: &Registry,
    surface_only: Option<&Registry>,
    recorded_specs: &[crate::message::ToolSpec],
    calls: Vec<RecordedCall>,
    mode: OnDivergence,
    cancel: CancellationToken,
) -> Result<Registry> {
    Ok(replay_registry_reporting(
        recorded_tools,
        live,
        surface_only,
        recorded_specs,
        calls,
        mode,
        cancel,
    )?
    .0)
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
fn build_replay_registry(
    recorded_tools: &[String],
    live: &Registry,
    surface_only: Option<&Registry>,
    recorded_specs: &[crate::message::ToolSpec],
    state: Arc<Mutex<ReplayState>>,
    mode: OnDivergence,
    cancel: CancellationToken,
) -> Result<Registry> {
    let mut registry = Registry::new();
    for name in recorded_tools {
        // `Error` and not just `Stop`: the two run identically and differ only
        // in the policy the *caller* applies to the report, so gating on
        // `Stop` alone would leave one non-executing mode still bailing — a
        // guard that reads correct and is wrong for the one variant whose
        // difference is not behavioural.
        let executes = mode.executes();
        // The recorded spec, when the surface store still holds the blob the
        // recording cites. In the non-executing modes it does two jobs: it
        // overrides a live tool's *words* (re-describe is the drift that
        // happens constantly, and tools render at the very front of the
        // prompt), and it stands in bodily for a tool nothing today can
        // construct — a retired integration, a renamed MCP prefix — which
        // used to skip the probe outright. Under `Live` it does neither: the
        // tools genuinely run there, and they deserve their own words.
        let spec = recorded_spec_for(name, recorded_specs, executes);
        let stand_in = stand_in_for(name, surface_only, executes);
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

/// The recording's own spec for `name`, when the surface store still holds
/// the blob the recording cites. Never consulted under a mode that executes:
/// there the tools genuinely run and deserve their own words.
fn recorded_spec_for<'a>(
    name: &str,
    recorded_specs: &'a [crate::message::ToolSpec],
    executes: bool,
) -> Option<&'a crate::message::ToolSpec> {
    (!executes)
        .then(|| recorded_specs.iter().find(|s| s.name == name))
        .flatten()
}

/// The surface-only stand-in for `name` — `ask_user` and friends, which only
/// a front-end that owns a human ever registers.
fn stand_in_for<'a>(
    name: &str,
    surface_only: Option<&'a Registry>,
    executes: bool,
) -> Option<&'a Arc<dyn Tool>> {
    (!executes)
        .then(|| surface_only.and_then(|r| r.get(name)))
        .flatten()
}

/// The recorded tools no route can offer, in recorded order.
///
/// Resolution order is `build_replay_registry`'s, through the same two
/// helpers, because a preflight that disagrees with the build is worse than
/// no preflight: it either burns the model calls it was added to save or
/// refuses a probe that would have run. `the_preflight_matches_what_the_build_accepts`
/// pins the two together.
///
/// **A name none of the three routes can construct will not become
/// constructible on a later night.** Nothing about tomorrow's machine adds a
/// tool that no provider still serves and the recording never described — so
/// a caller should record such a probe as unmeasurable rather than retry it
/// nightly. Three of the seventeen held-out reflections on 2026-09-11 were in
/// exactly this state: recordings from before the surface store, citing
/// `pkg__kg_entity` and `google__calendar_create_event` from an MCP server
/// retired on 2026-09-04. They were re-probed every night to reach the same
/// refusal, each spending a ledger row that can never age and a `--cover` slot
/// that buys no coverage, and were reported as `retryable` — the "an outcome
/// that cannot be aged repeats forever" shape in a new costume. The refusal
/// itself was always free of provider calls: this resolution runs before
/// `provider::build`.
pub fn unconstructible_recorded_tools(
    recorded_tools: &[String],
    live: &Registry,
    surface_only: Option<&Registry>,
    recorded_specs: &[crate::message::ToolSpec],
    mode: OnDivergence,
) -> Vec<String> {
    let executes = mode.executes();
    recorded_tools
        .iter()
        .filter(|name| {
            live.get(name).is_none()
                && stand_in_for(name, surface_only, executes).is_none()
                && recorded_spec_for(name, recorded_specs, executes).is_none()
        })
        .cloned()
        .collect()
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
    /// How many of `replayed_calls` a branch's first regenerated turn made —
    /// the decision a branch point asks about, and the only turn whose world
    /// is surely the recording's: an `Arguments` divergence is not
    /// structural, so the run goes on and its next calls are answered with
    /// results recorded for calls nobody made (`counterfactual::
    /// repeat_verdict`). `None` for a whole replay, and for a branch whose
    /// first turn cannot be read back faithfully: a compaction rewrote the
    /// history, no turn was produced, or its calls do not open the trace.
    pub decision_turn_calls: Option<usize>,
}

impl ReplayReport {
    /// Why whole-trajectory statistics cannot be scored as a faithful replay.
    /// Missing trailing calls do not trip the cursor's cancellation token:
    /// inspect the completed diff as well as a policy stop.
    pub fn unmeasurable_reason(&self) -> Option<String> {
        self.structural()
            .next()
            .map(|d| format!("structural divergence at call #{}: {d:?}", d.index()))
            .or_else(|| {
                self.stopped_early
                    .then(|| "replay stopped before completion".into())
            })
    }

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
    trajectory.ensure_replayable()?;
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
        decision_turn_calls: None,
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
    trajectory.ensure_replayable()?;
    let base = call_base.min(trajectory.calls.len());
    // The prefix is history, not a new security boundary. Private capability
    // metadata was not recorded, so treat prior tool results conservatively
    // as private; external provenance distinguishes outside content from
    // known harness results. This especially matters for live divergence.
    let mut taint = crate::agent::Taint::default();
    for message in &seed {
        for block in &message.content {
            if let crate::message::Block::ToolResult { tool_use_id, .. } = block {
                taint.private = true;
                taint.untrusted |= message
                    .tool_provenance
                    .get(tool_use_id)
                    .copied()
                    .unwrap_or(true);
            }
        }
    }
    let seeded = seed.len();
    let mut convo = Conversation::resumed(seed, taint);
    let mut stats = crate::session::RunStats {
        usage_complete: true,
        ..Default::default()
    };
    let outcome = agent.run_in(cx, &mut convo, None).await?;
    stats.absorb(&outcome);
    let decision_turn_calls = decision_turn(
        &convo.messages,
        seeded,
        outcome.compactions,
        &outcome.tool_calls,
    );
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
        decision_turn_calls,
    })
}

/// How many calls the first turn a branch regenerated made — the first
/// assistant message past the `seeded` prefix — when that can be read back
/// faithfully; see [`ReplayReport::decision_turn_calls`]. Checked against the
/// trace rather than trusted: the turn's calls must be the trace's first
/// entries, name for name, or the count is `None`, never a guess.
///
/// **As a multiset, not in block order.** `agent::dispatch` traces a call
/// settled at the gate (denied, unknown, refused by the phase, staged)
/// while it approves, ahead of the calls it then executes, so a turn mixing
/// the two traces out of block order — and a positional check discarded
/// every such turn (found on review of #729). Every one of the turn's calls
/// is traced before the next turn's, so the first `n` entries are this
/// turn's, in some order; the window [`crate::counterfactual`] grades is a
/// set either way.
fn decision_turn(
    messages: &[Message],
    seeded: usize,
    compactions: u32,
    trace: &[crate::agent::ToolCallTrace],
) -> Option<usize> {
    if compactions > 0 {
        return None;
    }
    let turn = messages
        .get(seeded..)?
        .iter()
        .find(|m| m.role == crate::message::Role::Assistant && !m.harness)?;
    let uses = turn.tool_uses();
    // Names suffice: which calls the turn made is the question here, and
    // the verdict compares their inputs itself.
    let head = trace.get(..uses.len())?;
    let mut issued: Vec<&str> = uses.iter().map(|(_, name, _)| *name).collect();
    let mut traced: Vec<&str> = head.iter().map(|c| c.name.as_str()).collect();
    issued.sort_unstable();
    traced.sort_unstable();
    (issued == traced).then_some(uses.len())
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
                external: None,
                name: NeverRun.name().to_string(),
                input: json!({}),
                output: "the recorded answer".into(),
                is_error: false,
                batch: None,
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
        // nothing where the recording ran something. Still fatal: the live
        // constructor offers no stand-ins at all.
        let err = replay_registry_live(
            &recorded,
            &live,
            &[],
            Vec::new(),
            LiveGate::reads_only(),
            CancellationToken::new(),
        )
        .err()
        .expect("a stand-in must not satisfy a live replay");
        assert!(!err.to_string().contains("needs a gate"), "{err}");

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

    #[tokio::test]
    async fn replay_preserves_external_results_and_refusals_and_keeps_unknown_untrusted() {
        for external in [Some(true), Some(false), None] {
            let cancel = CancellationToken::new();
            let mut call = recorded("echo", json!({}), "recorded bytes");
            call.external = external;
            let registry = replay_reg(vec![call], OnDivergence::Stop, &cancel);
            let output = registry
                .get("echo")
                .unwrap()
                .call(json!({}), &ToolCtx::default())
                .await
                .unwrap();
            assert_eq!(output.external, external.unwrap_or(true));
            assert_eq!(output.content, "recorded bytes");
        }
    }

    #[tokio::test]
    async fn reordered_batch_keeps_provenance_with_the_matched_arguments() {
        let cancel = CancellationToken::new();
        let mut outside = batched(0, "echo", json!({"source": "outside"}), "outside bytes");
        outside.external = Some(true);
        let mut harness = batched(0, "echo", json!({"source": "harness"}), "harness refusal");
        harness.external = Some(false);
        let registry = replay_reg(vec![outside, harness], OnDivergence::Stop, &cancel);
        let tool = registry.get("echo").unwrap();
        for (source, content, external) in [
            ("harness", "harness refusal", false),
            ("outside", "outside bytes", true),
        ] {
            let output = tool
                .call(json!({"source": source}), &ToolCtx::default())
                .await
                .unwrap();
            assert_eq!(output.content, content);
            assert_eq!(output.external, external);
        }
        assert!(!cancel.is_cancelled());
    }

    /// A recorded call with no batch marker — its own batch of one, and so the
    /// strict matching. Kept for the tests written against that behaviour; the
    /// batch-tolerance tests say what they mean with [`batched`].
    fn recorded(name: &str, input: Value, output: &str) -> RecordedCall {
        RecordedCall {
            external: None,
            name: name.into(),
            input,
            output: output.into(),
            is_error: false,
            batch: None,
        }
    }

    /// A recorded call issued as part of batch `batch`.
    fn batched(batch: u32, name: &str, input: Value, output: &str) -> RecordedCall {
        RecordedCall {
            batch: Some(batch),
            ..recorded(name, input, output)
        }
    }

    fn live_reg(
        calls: Vec<RecordedCall>,
        gate: LiveGate,
        cancel: &CancellationToken,
    ) -> (Registry, DivergenceProbe) {
        replay_registry_live(
            &["echo".to_string(), "other".to_string()],
            &live_registry(),
            &[],
            calls,
            gate,
            cancel.clone(),
        )
        .unwrap()
    }

    /// A gate that guards `echo` the way an operator's config would.
    fn gate_routing(names: &[&str]) -> LiveGate {
        LiveGate::new(
            names.iter().map(|n| n.to_string()),
            Arc::new(crate::policy::ExecPolicy::empty()),
            Arc::new(crate::hooks::HookSet::default()),
        )
    }

    /// Review of #713: `READ_ONLY_WRITERS` is the gate's one hand-maintained
    /// fact, and `Tool::read_only` defaults to `false`, so the gate's safety
    /// rests on "every built-in answering read-only is either a pure read or
    /// in that list". This pins every non-test `read_only()` whose body is
    /// literally `true`, by file **and implementing type**, each with its
    /// classification. Adding, renaming, moving or swapping one fails here
    /// until its author decides which it is (a count per file would let a
    /// writer replace a read in the same file unnoticed).
    ///
    /// Not covered: a `read_only()` that *computes* its answer. The gate
    /// trusts it as a real run's approver does; ROLLOUT's closed effect class
    /// is what replaces this list and this test.
    ///
    /// `PURE` entries are refused anyway when their capabilities say so:
    /// `WebSearch`/`WebOpen` for egress, `Subagent` for starting a child run,
    /// `RequestAccess` for declaring `destructive` (a yes widens the jail).
    /// `WRITES` entries must name a tool in `READ_ONLY_WRITERS`.
    #[test]
    fn every_read_only_tool_in_the_tree_is_classified() {
        const PURE: bool = false;
        const WRITES: bool = true;
        // (file, implementing type, registered name if a literal, class)
        const PINNED: &[(&str, &str, Option<&str>, bool)] = &[
            (
                "mecha-core/src/tool/request_access.rs",
                "RequestAccess",
                None,
                PURE,
            ),
            (
                "mecha-cli/src/slack/show.rs",
                "ShowFileTool",
                Some("show_file"),
                WRITES,
            ),
            ("mecha-core/src/gossip.rs", "GraphTool", None, PURE),
            (
                "mecha-core/src/gossip.rs",
                "LensedSearch",
                Some("kg_search"),
                PURE,
            ),
            (
                "mecha-core/src/imagegen.rs",
                "ImageGenerate",
                Some("image_generate"),
                WRITES,
            ),
            (
                "mecha-core/src/mailbox.rs",
                "MessageSendTool",
                Some("message_send"),
                WRITES,
            ),
            (
                "mecha-core/src/persona/files.rs",
                "FileRead",
                Some("file_read"),
                PURE,
            ),
            (
                "mecha-core/src/persona/memory_tools.rs",
                "MemoryRead",
                Some("memory_read"),
                PURE,
            ),
            (
                "mecha-core/src/persona/memory_tools.rs",
                "MemorySearch",
                Some("memory_search"),
                PURE,
            ),
            (
                "mecha-core/src/persona/search.rs",
                "FileSearch",
                Some("file_search"),
                PURE,
            ),
            (
                "mecha-core/src/search.rs",
                "WebOpen",
                Some("web_open"),
                PURE,
            ),
            (
                "mecha-core/src/search.rs",
                "WebSearch",
                Some("web_search"),
                PURE,
            ),
            ("mecha-core/src/subagent.rs", "Subagent", None, PURE),
            (
                "mecha-core/src/tool/ask.rs",
                "AskUserTool",
                Some("ask_user"),
                WRITES,
            ),
            (
                "mecha-core/src/tool/builtin.rs",
                "CompactTool",
                Some("compact"),
                PURE,
            ),
            (
                "mecha-core/src/tool/builtin.rs",
                "FsList",
                Some("fs_list"),
                PURE,
            ),
            (
                "mecha-core/src/tool/builtin.rs",
                "FsRead",
                Some("fs_read"),
                PURE,
            ),
            (
                "mecha-core/src/tool/builtin.rs",
                "HttpFetch",
                Some("http_fetch"),
                WRITES,
            ),
            (
                "mecha-core/src/tool/document.rs",
                "DocumentRead",
                Some("document_read"),
                PURE,
            ),
            (
                "mecha-core/src/tool/goal_context.rs",
                "GoalContext",
                Some("goal_context"),
                PURE,
            ),
            (
                "mecha-core/src/tool/hud.rs",
                "HudPreview",
                Some("hud_preview"),
                PURE,
            ),
            (
                "mecha-core/src/tool/hud.rs",
                "HudPropose",
                Some("hud_propose"),
                WRITES,
            ),
            (
                "mecha-core/src/tool/hud.rs",
                "HudSources",
                Some("hud_sources"),
                PURE,
            ),
            (
                "mecha-core/src/tool/image_library.rs",
                "ImageLibrary",
                Some("image_library"),
                PURE,
            ),
            (
                "mecha-core/src/tool/image_library.rs",
                "ImageLibraryPropose",
                Some("image_library_propose"),
                WRITES,
            ),
            (
                "mecha-core/src/tool/image_library.rs",
                "ImageLibraryReference",
                Some("image_library_reference"),
                WRITES,
            ),
            (
                "mecha-core/src/tool/image_view.rs",
                "ImageView",
                Some("image_view"),
                PURE,
            ),
            (
                "mecha-core/src/tool/persona_propose.rs",
                "PersonaPropose",
                Some("persona_propose"),
                WRITES,
            ),
            (
                "mecha-core/src/tool/recall.rs",
                "Recall",
                Some("recall"),
                PURE,
            ),
            (
                "mecha-core/src/tool/skill.rs",
                "SkillTool",
                Some("skill"),
                PURE,
            ),
            (
                "mecha-core/src/tool/system_read.rs",
                "SystemRead",
                Some("system_read"),
                PURE,
            ),
            (
                "mecha-core/src/tool/todo.rs",
                "TodoTool",
                Some("todo"),
                PURE,
            ),
        ];
        const KEY: &str = "fn read_only(&self) -> bool {";

        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        // Everything ahead of the file's first *inline* test module. A
        // `#[cfg(test)]` on a lone helper does not end the production code,
        // and neither does `#[cfg(test)] mod foo;`: that declares a sibling
        // file, and treating it as the cut would hide the rest of the
        // declaring file (`learning.rs` would end at line 57 of 8763).
        fn production(src: &str) -> &str {
            let mut from = 0;
            while let Some(at) = src[from..].find("#[cfg(test)]") {
                let at = from + at;
                let rest = src[at + "#[cfg(test)]".len()..].trim_start();
                let inline_mod = ["mod ", "pub mod ", "pub(crate) mod "]
                    .iter()
                    .filter_map(|kw| rest.strip_prefix(kw))
                    .any(|tail| {
                        tail.split_once('{')
                            .is_some_and(|(name, _)| !name.contains(';'))
                    });
                if inline_mod {
                    return &src[..at];
                }
                from = at + 1;
            }
            src
        }
        // The `impl … for X` header ahead of the method: the nearest *line*
        // that starts with `impl` and names a `for`, so a doc comment that
        // says "implementation" in between cannot be mistaken for it.
        fn impl_start(code: &str, at: usize) -> usize {
            let mut end = at;
            loop {
                let line_start = code[..end].rfind('\n').map_or(0, |i| i + 1);
                let line = code[line_start..end].trim_start();
                if line.starts_with("impl") && line.contains(" for ") {
                    return line_start;
                }
                assert!(line_start > 0, "read_only outside an impl");
                end = line_start - 1;
            }
        }
        // The registered name, when `fn name` returns a literal; `None` for
        // a name built at runtime (a subagent's profile, the graph's tools).
        fn registered_name(code: &str, imp: usize) -> Option<String> {
            const NAME: &str = "fn name(&self) -> &str {";
            let at = imp + code[imp..].find(NAME)? + NAME.len();
            let body = code[at..at + code[at..].find('}')?].trim();
            (body.starts_with('"') && body.ends_with('"') && body.len() >= 2)
                .then(|| body[1..body.len() - 1].to_string())
        }
        fn implementing_type(code: &str, imp: usize) -> String {
            let head = &code[imp..imp + code[imp..].find('{').unwrap()];
            let after = head.rsplit(" for ").next().unwrap().trim();
            after
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap()
                .to_string()
        }

        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let mut found: std::collections::BTreeSet<(String, String, Option<String>)> =
            Default::default();
        for krate in ["mecha-core", "mecha-cli", "mecha-mail", "mecha-slack"] {
            let mut files = Vec::new();
            walk(&workspace.join(krate).join("src"), &mut files);
            for file in files {
                let text = std::fs::read_to_string(&file).unwrap();
                let code = production(&text);
                let mut from = 0;
                while let Some(at) = code[from..].find(KEY) {
                    let at = from + at;
                    let body_start = at + KEY.len();
                    let body_end = body_start + code[body_start..].find('}').unwrap();
                    let lines: Vec<&str> = code[body_start..body_end]
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with("//"))
                        .collect();
                    if lines == ["true"] {
                        let rel = file.strip_prefix(workspace).unwrap();
                        let imp = impl_start(code, at);
                        found.insert((
                            rel.to_string_lossy().replace('\\', "/"),
                            implementing_type(code, imp),
                            registered_name(code, imp),
                        ));
                    }
                    from = body_end;
                }
            }
        }
        let pinned: std::collections::BTreeSet<(String, String, Option<String>)> = PINNED
            .iter()
            .map(|(f, t, n, _)| (f.to_string(), t.to_string(), n.map(String::from)))
            .collect();
        let added: Vec<_> = found.difference(&pinned).collect();
        let gone: Vec<_> = pinned.difference(&found).collect();
        assert!(
            added.is_empty() && gone.is_empty(),
            "read_only() -> true changed: added {added:?}, gone {gone:?}. Decide \
             whether each added tool is a pure read or writes something (a store, a \
             file, a job, a post); add a writer's name to READ_ONLY_WRITERS; then \
             update PINNED"
        );
        // The join the gate depends on: a writer is refused by its registered
        // *name*, so the WRITES entries' names must be exactly the list. A
        // renamed writer changes `found` above and fails there.
        let writer_names: std::collections::BTreeSet<&str> = PINNED
            .iter()
            .filter(|(_, _, _, w)| *w)
            .map(|(_, t, n, _)| n.unwrap_or_else(|| panic!("writer {t} has no literal name")))
            .collect();
        let listed: std::collections::BTreeSet<&str> = READ_ONLY_WRITERS.iter().copied().collect();
        assert_eq!(
            writer_names, listed,
            "READ_ONLY_WRITERS must be exactly the registered names of the WRITES entries"
        );

        // The one PURE entry with a runtime name and no second net
        // (`Subagent` has `runs_a_fresh_conversation`): `GraphTool`'s name is
        // a private field its public constructors fix. Pin those literals, so
        // a constructor added for a graph *write* fails here.
        let gossip = std::fs::read_to_string(workspace.join("mecha-core/src/gossip.rs")).unwrap();
        let block_start = gossip.find("\nimpl GraphTool {").expect("impl GraphTool");
        let block = &gossip[block_start..];
        let block = &block[..block.find("\n}\n").unwrap()];
        let graph_names: std::collections::BTreeSet<&str> = block
            .match_indices("name: \"")
            .map(|(at, m)| {
                let rest = &block[at + m.len()..];
                &rest[..rest.find('"').unwrap()]
            })
            .collect();
        assert_eq!(
            graph_names,
            ["kg_entity", "kg_search", "kg_verify"]
                .into_iter()
                .collect(),
            "GraphTool answers read_only() for every name it is built with; a new \
             constructor must be a read, or GraphTool must stop answering read-only"
        );
    }

    /// Review of #713: a live replay without a gate is refused where it is
    /// built, so a missing gate can never be reported as the branch's own
    /// first consequential call.
    #[test]
    fn a_live_replay_cannot_be_built_without_a_gate() {
        let err = replay_registry_reporting(
            &["echo".to_string(), "other".to_string()],
            &live_registry(),
            None,
            &[],
            vec![recorded("echo", json!({}), "recorded")],
            OnDivergence::Live,
            CancellationToken::new(),
        )
        .err()
        .expect("an ungated live replay must not build");
        assert!(err.to_string().contains("needs a gate"), "{err}");
    }

    /// The defect `docs/ARCHITECTURE.md` §Session records and replay names: past the divergence a
    /// routed call ran. It now stops in front of it, records which call and
    /// why, and executes nothing.
    #[tokio::test]
    async fn the_gate_stops_before_a_routed_call_and_names_it() {
        let cancel = CancellationToken::new();
        let (reg, probe) = live_reg(
            vec![recorded("echo", json!({}), "recorded")],
            gate_routing(&["other"]),
            &cancel,
        );
        let out = reg
            .get("other")
            .unwrap()
            .call(json!({"to": "someone"}), &ToolCtx::default())
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(
            out.content.contains("stopped before `other`"),
            "{}",
            out.content
        );
        assert!(
            cancel.is_cancelled(),
            "the run ends at the first consequential call"
        );
        assert_eq!(
            probe.halted(),
            vec![Halted {
                tool: "other".into(),
                input: json!({"to": "someone"}),
                reason: "the outbox routes it".into(),
            }]
        );
    }

    /// What the gate refuses, one reason each: a write, a send, a known
    /// read-only writer, an approval rule, a `pre_tool` hook. A pure read with
    /// nothing configured runs.
    #[test]
    fn the_gate_refuses_everything_but_an_unguarded_pure_read() {
        struct Named(&'static str, bool, Egress);
        #[async_trait]
        impl Tool for Named {
            fn name(&self) -> &str {
                self.0
            }
            fn description(&self) -> &str {
                "test"
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object"})
            }
            fn read_only(&self) -> bool {
                self.1
            }
            fn capabilities(&self) -> Capabilities {
                Capabilities {
                    egress: self.2,
                    ..Default::default()
                }
            }
            async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
                Ok(ToolOutput::ok("ran"))
            }
        }
        let open = LiveGate::reads_only();
        let read = Named("fs_read", true, Egress::None);
        assert_eq!(open.refuses(&read, &json!({})), None);

        // Review of #713: a subagent reports read-only with no egress when
        // its child cannot send, and still writes through the child.
        struct Delegates;
        #[async_trait]
        impl Tool for Delegates {
            fn name(&self) -> &str {
                "editor"
            }
            fn description(&self) -> &str {
                "test"
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object"})
            }
            fn read_only(&self) -> bool {
                true
            }
            fn runs_a_fresh_conversation(&self) -> bool {
                true
            }
            async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
                Ok(ToolOutput::ok("ran"))
            }
        }
        assert!(open
            .refuses(&Delegates, &json!({}))
            .unwrap()
            .contains("child run"));
        assert!(open
            .refuses(&Named("fs_write", false, Egress::None), &json!({}))
            .unwrap()
            .contains("writes"));
        assert!(open
            .refuses(&Named("web_search", true, Egress::Blind), &json!({}))
            .unwrap()
            .contains("send"));
        assert!(open
            .refuses(&Named("message_send", true, Egress::None), &json!({}))
            .unwrap()
            .contains("reports itself read-only"));

        let hooked = LiveGate::new(
            Vec::new(),
            Arc::new(crate::policy::ExecPolicy::empty()),
            Arc::new(
                crate::hooks::HookSet::from_config(&[crate::config::HookConfig {
                    event: "pre_tool".into(),
                    command: "true".into(),
                    tools: vec!["fs_read".into()],
                    timeout_secs: None,
                }])
                .unwrap(),
            ),
        );
        assert!(hooked
            .refuses(&read, &json!({}))
            .unwrap()
            .contains("pre_tool hook"));
        let observed = LiveGate::new(
            Vec::new(),
            Arc::new(crate::policy::ExecPolicy::empty()),
            Arc::new(
                crate::hooks::HookSet::from_config(&[crate::config::HookConfig {
                    event: "post_tool".into(),
                    command: "true".into(),
                    tools: vec!["fs_read".into()],
                    timeout_secs: None,
                }])
                .unwrap(),
            ),
        );
        assert_eq!(
            observed.refuses(&read, &json!({})),
            None,
            "an observer guards nothing"
        );

        #[derive(serde::Deserialize)]
        struct W {
            rule: Vec<crate::policy::RuleConfig>,
        }
        let w: W = toml::from_str(
            r#"
[[rule]]
tool = "fs_read"
decision = "prompt"
"#,
        )
        .unwrap();
        let ruled = LiveGate::new(
            Vec::new(),
            Arc::new(crate::policy::ExecPolicy::from_config(&w.rule, true).unwrap()),
            Arc::new(crate::hooks::HookSet::default()),
        );
        assert!(ruled
            .refuses(&read, &json!({}))
            .unwrap()
            .contains("approval rule"));
    }

    /// Review of #713's first shape, which put the operator's outbox route on
    /// the run: staging acts ahead of the tool, so a routed call replayed
    /// *before* the divergence never reached the cursor, and the replay died
    /// one call later and ran the rest live. The gate is asked only of calls
    /// that would genuinely run, so a recording full of routed calls that the
    /// model reproduces replays to its end and stops in front of nothing.
    #[tokio::test]
    async fn routed_calls_before_the_divergence_still_replay_from_the_recording() {
        let calls = vec![
            batched(0, "echo", json!({"value": "a"}), "a"),
            batched(1, "echo", json!({"value": "b"}), "b"),
        ];
        let trajectory = Trajectory {
            turns: vec!["both calls".into()],
            calls: calls.clone(),
            ..Default::default()
        };
        let cancel = CancellationToken::new();
        let (registry, probe) = live_reg(calls, gate_routing(&["echo"]), &cancel);
        let approver = Arc::new(ModeApprover {
            mode: PermissionMode::Allow,
        });
        let agent = Agent::new(
            Box::new(Scripted(Mutex::new(vec![
                assistant(
                    vec![tool_use("a", "echo", json!({"value": "a"}))],
                    StopReason::ToolUse,
                ),
                assistant(
                    vec![tool_use("b", "echo", json!({"value": "b"}))],
                    StopReason::ToolUse,
                ),
                assistant(vec![Block::text("done")], StopReason::EndTurn),
            ]))),
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
        let report = drive(&agent, &cx, &trajectory).await.unwrap();
        assert!(report.divergences.is_empty(), "{:?}", report.divergences);
        assert!(probe.halted().is_empty());
        assert_eq!(probe.reason(), None);
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
                external: None,
                name: "echo".into(),
                input: json!({}),
                output: "no such file".into(),
                is_error: true,
                batch: None,
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
        let reg = live_reg(
            vec![recorded("echo", json!({}), "recorded")],
            LiveGate::reads_only(),
            &cancel,
        )
        .0;
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
                    egress: Egress::Chosen,
                    ..Default::default()
                }
            }
            async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
                Ok(ToolOutput::ok("sent"))
            }
        }

        let mut live = Registry::new();
        live.insert(Arc::new(Sender));
        let caps_under = |mode: OnDivergence| {
            let reg = if mode.executes() {
                replay_registry_live(
                    &["mail_send".to_string()],
                    &live,
                    &[],
                    Vec::new(),
                    LiveGate::reads_only(),
                    CancellationToken::new(),
                )
                .unwrap()
                .0
            } else {
                replay_registry(
                    &["mail_send".to_string()],
                    &live,
                    None,
                    &[],
                    Vec::new(),
                    mode,
                    CancellationToken::new(),
                )
                .unwrap()
            };
            reg.get("mail_send").unwrap().capabilities()
        };

        for mode in [OnDivergence::Stop, OnDivergence::Error] {
            let caps = caps_under(mode);
            assert!(!caps.can_send(), "a replayed send sends nothing");
            assert!(caps.private_data, "private data must not narrow");
        }
        // Under Live the tool genuinely runs, and the interlock deserves the
        // truth about it.
        assert_eq!(caps_under(OnDivergence::Live).egress, Egress::Chosen);
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
                external: None,
                name: recorded_spec.name.clone(),
                input: json!({"name": "Yuqi"}),
                output: "the recorded entity".into(),
                is_error: false,
                batch: None,
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
            !tool.capabilities().can_send(),
            "a replayed call sends nothing"
        );
        // And answered from the recording, never executed.
        let out = tool
            .call(json!({"name": "Yuqi"}), &ToolCtx::default())
            .await
            .unwrap();
        assert_eq!(out.content, "the recorded entity");

        // Live mode executes, and a spec cannot: still fatal.
        let err = replay_registry_live(
            &names,
            &Registry::new(),
            &specs,
            Vec::new(),
            LiveGate::reads_only(),
            CancellationToken::new(),
        )
        .err()
        .expect("a spec alone must not satisfy a live replay");
        assert!(!err.to_string().contains("needs a gate"), "{err}");
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

        let (live, _) = replay_registry_live(
            &names,
            &live_registry(),
            &specs,
            Vec::new(),
            LiveGate::reads_only(),
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
    async fn a_missing_tail_is_unmeasurable_without_cursor_cancellation() {
        let calls = vec![
            batched(0, "echo", json!({"value":"a"}), "a"),
            batched(1, "echo", json!({"value":"b"}), "b"),
        ];
        let trajectory = Trajectory {
            turns: vec!["both calls".into()],
            calls: calls.clone(),
            ..Default::default()
        };
        let report = drive_scripted(
            vec![
                assistant(
                    vec![tool_use("a", "echo", json!({"value":"a"}))],
                    StopReason::ToolUse,
                ),
                assistant(vec![Block::text("done")], StopReason::EndTurn),
            ],
            calls,
            &trajectory,
        )
        .await;
        assert!(
            !report.stopped_early,
            "the cursor never saw an unmatched call"
        );
        assert!(report.unmeasurable_reason().unwrap().contains("Missing"));
    }

    #[tokio::test]
    async fn harness_observations_are_refused_before_either_replay_driver_calls_a_provider() {
        let approver = Arc::new(ModeApprover {
            mode: PermissionMode::Allow,
        });
        let agent = Agent::new(
            Box::new(Scripted(Mutex::new(Vec::new()))),
            Registry::new(),
            approver.clone(),
            ToolCtx::default(),
            AgentConfig::default(),
            None,
        )
        .unwrap();
        let cx = RunContext::new(ToolCtx::default(), approver);
        let trajectory = Trajectory {
            turns: vec!["task".into()],
            harness_calls: 1,
            ..Default::default()
        };
        let error = drive(&agent, &cx, &trajectory).await.unwrap_err();
        assert!(error.to_string().contains("replay of check observations"));
        let error = drive_branch(&agent, &cx, vec![Message::user("task")], &trajectory, 0)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("replay of check observations"));
    }

    #[tokio::test]
    async fn a_faithful_replay_reports_no_divergence() {
        let calls = vec![recorded("echo", json!({"value": "a"}), "first")];
        let trajectory = Trajectory {
            turns: vec!["do the thing".into()],
            calls: calls.clone(),
            final_text: "done".into(),
            steered: false,
            harness_calls: 0,
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
    async fn an_image_only_branch_seed_blocks_a_live_send_after_external_content() {
        struct BoundaryTool {
            name: &'static str,
            external: bool,
        }
        #[async_trait]
        impl Tool for BoundaryTool {
            fn name(&self) -> &str {
                self.name
            }
            fn description(&self) -> &str {
                "Test a replay boundary"
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object"})
            }
            fn read_only(&self) -> bool {
                true
            }
            fn capabilities(&self) -> Capabilities {
                if self.external {
                    Capabilities::default().untrusted()
                } else {
                    Capabilities::default().sends()
                }
            }
            async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
                assert!(self.external, "image-bearing replay executed a live send");
                Ok(ToolOutput::ok("external bytes").from_outside())
            }
        }
        let mut live = Registry::new();
        live.insert(Arc::new(BoundaryTool {
            name: "fetch",
            external: true,
        }));
        live.insert(Arc::new(BoundaryTool {
            name: "send",
            external: false,
        }));
        let cancel = CancellationToken::new();
        // A reads-only gate lets `fetch` run; `send` is refused by the
        // interlock in the loop, ahead of the gate, which never sees it.
        let (registry, _) = replay_registry_live(
            &["fetch".into(), "send".into()],
            &live,
            &[],
            vec![],
            LiveGate::reads_only(),
            cancel.clone(),
        )
        .unwrap();
        let approver = Arc::new(ModeApprover {
            mode: PermissionMode::Allow,
        });
        let agent = Agent::new(
            Box::new(Scripted(Mutex::new(vec![
                assistant(
                    vec![tool_use("t1", "fetch", json!({}))],
                    StopReason::ToolUse,
                ),
                assistant(vec![tool_use("t2", "send", json!({}))], StopReason::ToolUse),
                assistant(vec![Block::text("done")], StopReason::EndTurn),
            ]))),
            registry,
            approver.clone(),
            ToolCtx::default(),
            AgentConfig::default(),
            None,
        )
        .unwrap();
        let seed = vec![Message {
            role: crate::message::Role::User,
            content: vec![Block::image(
                "image/png",
                b"pixels",
                Some("shot.png".into()),
            )],
            harness: false,
            planning: None,
            tool_provenance: Default::default(),
        }];
        let cx = RunContext::new(ToolCtx::default(), approver).with_cancel(cancel);
        let report = drive_branch(&agent, &cx, seed, &Trajectory::default(), 0)
            .await
            .unwrap();
        assert_eq!(report.stats.blocked_sends, 1);
    }

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
            harness_calls: 0,
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

    /// The regression, at the driver. The model issued the recorded batch in
    /// the other order — the same two calls, which the loop runs concurrently
    /// anyway — and the replay must not stop. Before this the cursor compared
    /// against one position and cancelled at call #0; four of the twelve
    /// episodes the 2026-09-08 harness pass dropped died exactly here, on
    /// batches they had reproduced exactly.
    #[tokio::test]
    async fn a_batch_replayed_in_another_order_does_not_stop_the_replay() {
        let calls = vec![
            batched(0, "echo", json!({"value": "a"}), "first"),
            batched(0, "other", json!({}), "second"),
        ];
        let trajectory = Trajectory {
            turns: vec!["do the thing".into()],
            calls: calls.clone(),
            final_text: "done".into(),
            steered: false,
            harness_calls: 0,
        };

        let report = drive_scripted(
            vec![
                assistant(
                    vec![
                        tool_use("t1", "other", json!({})),
                        tool_use("t2", "echo", json!({"value": "a"})),
                    ],
                    StopReason::ToolUse,
                ),
                assistant(vec![Block::text("done")], StopReason::EndTurn),
            ],
            calls,
            &trajectory,
        )
        .await;

        assert!(!report.stopped_early, "{:?}", report.divergences);
        assert!(report.divergences.is_empty(), "{:?}", report.divergences);
        assert_eq!(report.replayed_calls.len(), 2);
        assert_eq!(report.final_text, "done");
    }

    /// And each call is answered with the result recorded for *it*, not with
    /// whatever sat at the cursor. Getting this wrong would be worse than
    /// stopping: the run continues on another call's output.
    #[tokio::test]
    async fn a_reordered_batch_answers_each_call_with_its_own_output() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![
                batched(0, "echo", json!({"value": "a"}), "first"),
                batched(0, "other", json!({}), "second"),
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
        assert_eq!(out.content, "second");

        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({"value": "a"}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "first");
        assert!(!cancel.is_cancelled(), "a reordered batch is not a fork");
    }

    /// A batch that calls one tool twice is answered by *arguments*, not by
    /// position. `decide` and `replay::pair_batch` must choose the same mate:
    /// pairing by name here while the diff pairs by arguments hands the run
    /// each call's sibling output and then reports the episode clean — worse
    /// than the strict cursor this replaced, which at least reported two
    /// argument divergences over the same wrong outputs.
    #[tokio::test]
    async fn a_reordered_batch_of_one_tool_is_answered_by_arguments() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![
                batched(0, "echo", json!({"value": "a"}), "A"),
                batched(0, "echo", json!({"value": "b"}), "B"),
            ],
            OnDivergence::Stop,
            &cancel,
        );
        let ctx = ToolCtx::default();

        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({"value": "b"}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "B", "answered with its sibling's output");

        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({"value": "a"}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "A", "answered with its sibling's output");
        assert!(!cancel.is_cancelled());
    }

    /// The other half of `a_repeated_tool_pairs_the_way_decide_spent_the_recording`:
    /// what `decide` actually feeds in that case. The two must agree, and this
    /// is the side that decides what the model reads.
    #[tokio::test]
    async fn a_repeated_tool_with_a_changed_argument_spends_the_recording_in_order() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![
                batched(0, "echo", json!({"value": "a"}), "A"),
                batched(0, "echo", json!({"value": "b"}), "B"),
            ],
            OnDivergence::Stop,
            &cancel,
        );
        let ctx = ToolCtx::default();

        // No exact match, so the fallback spends recorded #0.
        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({"value": "z"}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "A");

        // #0 is gone, so the call that *did* reproduce it gets #1 — which the
        // diff reports at both positions rather than clearing this one.
        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({"value": "a"}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "B");
        assert!(!cancel.is_cancelled());
    }

    /// The fallback the arguments pass sits in front of: an argument that
    /// genuinely changed still gets the recorded answer, because a path spelled
    /// differently is the same decision and the diff is what reports it.
    #[tokio::test]
    async fn a_changed_argument_still_gets_the_recorded_answer() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![batched(0, "echo", json!({"value": "a"}), "A")],
            OnDivergence::Stop,
            &cancel,
        );

        let out = reg
            .get("echo")
            .unwrap()
            .call(json!({"value": "z"}), &ToolCtx::default())
            .await
            .unwrap();
        assert_eq!(out.content, "A");
        assert!(!cancel.is_cancelled());
    }

    /// The tolerance stops at the batch edge. `other` belongs to the next
    /// turn, chosen after seeing this turn's results, so reaching for it early
    /// is a real divergence and still stops the run.
    #[tokio::test]
    async fn a_call_from_the_next_batch_still_stops_the_replay() {
        let cancel = CancellationToken::new();
        let reg = replay_reg(
            vec![
                batched(0, "echo", json!({"value": "a"}), "first"),
                batched(1, "other", json!({}), "second"),
            ],
            OnDivergence::Stop,
            &cancel,
        );

        let out = reg
            .get("other")
            .unwrap()
            .call(json!({}), &ToolCtx::default())
            .await
            .unwrap();

        assert!(out.is_error, "{out:?}");
        assert!(out.content.contains("recorded call #0"), "{}", out.content);
        assert!(cancel.is_cancelled());
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
            harness_calls: 0,
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

#[cfg(test)]
mod surface_loss_tests {
    use super::*;
    use crate::message::ToolSpec;
    use crate::tool::{Registry, Tool, ToolCtx, ToolOutput};
    use async_trait::async_trait;
    use serde_json::json;

    struct Named(&'static str);
    #[async_trait]
    impl Tool for Named {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "t"
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn read_only(&self) -> bool {
            true
        }
        async fn call(&self, _i: Value, _c: &ToolCtx) -> Result<ToolOutput> {
            Ok(ToolOutput::ok("x"))
        }
    }

    fn registry(names: &[&'static str]) -> Registry {
        let mut r = Registry::new();
        for n in names {
            r.insert(Arc::new(Named(n)));
        }
        r
    }

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: "recorded".into(),
            input_schema: json!({"type": "object"}),
        }
    }

    /// The case the fix exists for: a recording from before the surface store
    /// that names a tool from a retired MCP server. No route can build it, and
    /// no later night changes that.
    #[test]
    fn a_retired_tool_with_no_surface_blob_is_unconstructible() {
        let lost = unconstructible_recorded_tools(
            &["pkg__kg_entity".into(), "fs_read".into()],
            &registry(&["fs_read"]),
            None,
            &[],
            OnDivergence::Stop,
        );
        assert_eq!(lost, vec!["pkg__kg_entity".to_string()]);
    }

    /// The same name is constructible when the recording described it: the
    /// blob stands in bodily, which is why the 2026-09-04 recording graded
    /// while the August ones could not.
    #[test]
    fn a_recorded_spec_makes_a_retired_tool_constructible() {
        let lost = unconstructible_recorded_tools(
            &["pkg__kg_entity".into()],
            &registry(&[]),
            None,
            &[spec("pkg__kg_entity")],
            OnDivergence::Stop,
        );
        assert!(lost.is_empty(), "the blob should stand in: {lost:?}");
    }

    /// A surface-only tool is not lost: `ask_user` is on the recorded surface
    /// of every interactive session and no CLI registry ever holds it.
    #[test]
    fn a_surface_only_tool_is_not_lost() {
        let lost = unconstructible_recorded_tools(
            &["ask_user".into()],
            &registry(&[]),
            Some(&registry(&["ask_user"])),
            &[],
            OnDivergence::Stop,
        );
        assert!(lost.is_empty(), "{lost:?}");
    }

    /// Under a mode that executes, neither the blob nor the stand-in may
    /// substitute — the tools genuinely run there — so the same recording is
    /// lost that `Stop` could have offered.
    #[test]
    fn an_executing_mode_cannot_substitute_a_blob() {
        let recorded = ["pkg__kg_entity".to_string()];
        let specs = [spec("pkg__kg_entity")];
        assert!(unconstructible_recorded_tools(
            &recorded,
            &registry(&[]),
            None,
            &specs,
            OnDivergence::Stop
        )
        .is_empty());
        assert_eq!(
            unconstructible_recorded_tools(
                &recorded,
                &registry(&[]),
                None,
                &specs,
                OnDivergence::Live
            ),
            vec!["pkg__kg_entity".to_string()]
        );
    }

    /// **The preflight and the build must agree.** A preflight that says yes
    /// where the build says no burns the model calls it was added to save; one
    /// that says no where the build says yes refuses a probe that would have
    /// run. Both answers come from the same two helpers, and this pins them
    /// across the whole matrix rather than trusting that they were written to
    /// match.
    #[test]
    fn the_preflight_matches_what_the_build_accepts() {
        let specs = [spec("blobbed")];
        for name in ["live_tool", "stood_in", "blobbed", "gone"] {
            for mode in [OnDivergence::Stop, OnDivergence::Error, OnDivergence::Live] {
                let recorded = [name.to_string()];
                let live = registry(&["live_tool"]);
                let surface_only = registry(&["stood_in"]);
                let preflight_says_lost = !unconstructible_recorded_tools(
                    &recorded,
                    &live,
                    Some(&surface_only),
                    &specs,
                    mode,
                )
                .is_empty();
                // Live builds only through its gated constructor, which
                // offers no stand-ins: the same answer the preflight gives.
                let build_failed = if mode.executes() {
                    replay_registry_live(
                        &recorded,
                        &live,
                        &specs,
                        Vec::new(),
                        LiveGate::reads_only(),
                        CancellationToken::new(),
                    )
                    .is_err()
                } else {
                    replay_registry(
                        &recorded,
                        &live,
                        Some(&surface_only),
                        &specs,
                        Vec::new(),
                        mode,
                        CancellationToken::new(),
                    )
                    .is_err()
                };
                assert_eq!(
                    preflight_says_lost, build_failed,
                    "disagreement on `{name}` under {mode:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod divergence_reason_tests {
    use super::*;
    use crate::tool::{Registry, Tool, ToolCtx, ToolOutput};
    use async_trait::async_trait;
    use serde_json::json;

    struct Named(&'static str);
    #[async_trait]
    impl Tool for Named {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "fixture"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        async fn call(&self, _i: serde_json::Value, _c: &ToolCtx) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput::ok("live"))
        }
    }

    /// **The regression.** The reason a replay left its recording was built
    /// in `ReplayTool::decide`, handed to the model in a refusal, and then
    /// dropped: `ArmOutcome.diverged` was a bare `bool`, so
    /// `mecha harness show` could name twelve dropped episodes and say
    /// nothing about how any of them diverged.
    ///
    /// Fails on the old behaviour, where nothing carried the string at all.
    #[tokio::test]
    async fn a_wrong_tool_records_which_call_and_which_tool() {
        let mut live = Registry::new();
        live.insert(Arc::new(Named("recorded_tool")));
        live.insert(Arc::new(Named("other_tool")));

        let (registry, probe) = replay_registry_reporting(
            &["recorded_tool".into(), "other_tool".into()],
            &live,
            None,
            &[],
            vec![RecordedCall {
                external: None,
                name: "recorded_tool".into(),
                input: json!({}),
                output: "recorded".into(),
                is_error: false,
                batch: None,
            }],
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .expect("both tools are live");

        // Nothing has diverged yet: the negative that keeps this honest.
        assert_eq!(probe.reason(), None);

        // The recorded call, in order — still no divergence.
        registry
            .get("recorded_tool")
            .unwrap()
            .call(json!({}), &ToolCtx::default())
            .await
            .unwrap();
        assert_eq!(
            probe.reason(),
            None,
            "following the recording is not a divergence"
        );

        // Now the wrong tool. The recording ended, so this is the
        // past-the-end arm.
        registry
            .get("other_tool")
            .unwrap()
            .call(json!({}), &ToolCtx::default())
            .await
            .unwrap();
        let why = probe.reason().expect("a divergence must record its reason");
        assert!(
            why.contains("the recording ended after 1 call(s)"),
            "the reason must say what happened: {why}"
        );
    }

    /// A call to a tool the recording did not expect names both tools, so a
    /// reader can see the shape of the divergence without the transcript.
    #[tokio::test]
    async fn a_substituted_tool_names_both_sides() {
        let mut live = Registry::new();
        live.insert(Arc::new(Named("expected")));
        live.insert(Arc::new(Named("actual")));
        let (registry, probe) = replay_registry_reporting(
            &["expected".into(), "actual".into()],
            &live,
            None,
            &[],
            vec![RecordedCall {
                external: None,
                name: "expected".into(),
                input: json!({}),
                output: "recorded".into(),
                is_error: false,
                batch: None,
            }],
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .unwrap();

        registry
            .get("actual")
            .unwrap()
            .call(json!({}), &ToolCtx::default())
            .await
            .unwrap();
        let why = probe.reason().expect("recorded");
        assert!(why.contains("recorded call #0"), "{why}");
        assert!(why.contains("`expected`"), "{why}");
        assert!(why.contains("`actual`"), "{why}");
    }

    /// The FIRST divergence is the one that matters — everything after it
    /// happens in a run that has already left the recording, so a later
    /// message must not overwrite the cause.
    #[tokio::test]
    async fn the_first_reason_is_the_one_kept() {
        let mut live = Registry::new();
        live.insert(Arc::new(Named("expected")));
        live.insert(Arc::new(Named("actual")));
        let (registry, probe) = replay_registry_reporting(
            &["expected".into(), "actual".into()],
            &live,
            None,
            &[],
            vec![RecordedCall {
                external: None,
                name: "expected".into(),
                input: json!({}),
                output: "recorded".into(),
                is_error: false,
                batch: None,
            }],
            OnDivergence::Stop,
            CancellationToken::new(),
        )
        .unwrap();

        let actual = registry.get("actual").unwrap();
        actual.call(json!({}), &ToolCtx::default()).await.unwrap();
        let first = probe.reason().expect("recorded");
        actual.call(json!({}), &ToolCtx::default()).await.unwrap();
        assert_eq!(probe.reason().as_deref(), Some(first.as_str()));
    }
}

#[cfg(test)]
mod decision_turn_tests {
    use super::*;
    use crate::agent::ToolCallTrace;
    use crate::message::Block;
    use serde_json::json;

    fn call(name: &str) -> ToolCallTrace {
        ToolCallTrace {
            name: name.into(),
            input: json!({}),
            is_error: false,
            denied: false,
            unknown: false,
            staged: false,
        }
    }

    fn turn(names: &[&str]) -> Message {
        Message::assistant(
            names
                .iter()
                .enumerate()
                .map(|(i, n)| Block::ToolUse {
                    id: format!("r{i}"),
                    name: (*n).into(),
                    input: json!({}),
                })
                .collect(),
        )
    }

    /// The first regenerated turn is read past the seed, harness turns
    /// skipped, and believed only when its calls open the trace: a
    /// compaction, a missing turn or a trace it does not open is `None`,
    /// never a count a repeat verdict would grade by.
    #[test]
    fn the_decision_turn_is_read_back_or_not_at_all() {
        let seed = vec![Message::user("total the quarter"), turn(&["fs_read"])];
        let mut harness = Message::assistant(vec![Block::text("(harness note)")]);
        harness.harness = true;
        let mut messages = seed.clone();
        messages.extend([harness, turn(&["fs_list", "fs_read"]), turn(&["todo"])]);
        let trace = [call("fs_list"), call("fs_read"), call("todo")];
        assert_eq!(decision_turn(&messages, seed.len(), 0, &trace), Some(2));
        assert_eq!(decision_turn(&messages, seed.len(), 1, &trace), None);
        assert_eq!(
            decision_turn(
                &messages,
                seed.len(),
                0,
                &[call("fs_read"), call("fs_list")]
            ),
            Some(2),
            "a gate-settled call is traced ahead of an executed one (review of #729)"
        );
        assert_eq!(
            decision_turn(&messages, seed.len(), 0, &[call("fs_list"), call("todo")]),
            None,
            "a trace this turn's calls do not open"
        );
        assert_eq!(decision_turn(&seed, seed.len(), 0, &[]), None, "no turn");
        let mut ended = seed.clone();
        ended.push(Message::assistant(vec![Block::text(
            "I could not find it.",
        )]));
        assert_eq!(decision_turn(&ended, seed.len(), 0, &[]), Some(0));
    }
}
