//! `mecha` — an agent harness for local models.

mod appraisal_probe;
mod approve;
mod closure_guard;
mod commands;
mod editor;
mod exe;
mod follow;
mod harness_probe;
mod interrupt;
mod lesson_pass;
mod logs;
mod pointwise_pass;
mod probe;
mod render;
mod review_policy;
mod setup;
mod slack;
mod success_readout;
#[cfg(test)]
mod testenv;
mod tui;
mod voice;

use anyhow::Result;
use clap::{Parser, Subcommand};
use mecha_core::message::Effort;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "mecha",
    version,
    about = "An agent harness for local models: one loop, any model, native and MCP tools.",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOpts,

    #[command(subcommand)]
    pub command: Command,
}

/// Options that apply to any command that actually runs an agent.
#[derive(clap::Args, Clone, Debug, Default)]
pub struct GlobalOpts {
    /// Provider to use, by config key (default: the config's default_provider).
    #[arg(long, short = 'p', global = true)]
    pub provider: Option<String>,

    /// Model id, overriding the provider's default.
    #[arg(long, short = 'm', global = true)]
    pub model: Option<String>,

    /// Reasoning depth: low, medium, high, xhigh, max.
    #[arg(long, short = 'e', global = true)]
    pub effort: Option<Effort>,

    /// System prompt. Use @path to read it from a file.
    #[arg(long, short = 's', global = true)]
    pub system: Option<String>,

    /// Directory the agent may read and write. Defaults to the working directory.
    #[arg(long, short = 'w', global = true)]
    pub workspace: Option<PathBuf>,

    /// Approve every tool call without asking. Required for unattended runs
    /// that need to write or execute anything.
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,

    /// Refuse anything that isn't read-only.
    #[arg(long, global = true, conflicts_with = "yes")]
    pub read_only: bool,

    /// Stop after this many model turns.
    #[arg(long, global = true)]
    pub max_turns: Option<u32>,

    /// Stop once the run has generated this many output tokens.
    #[arg(long, global = true)]
    pub max_output_tokens: Option<u64>,

    /// Stop once the run has cost this much, in USD. Needs prices configured
    /// on the provider.
    #[arg(long, global = true, value_name = "USD")]
    pub max_cost: Option<f64>,

    /// Only expose these tools (repeatable). Names are matched exactly.
    #[arg(long = "tool", global = true)]
    pub tools: Vec<String>,

    /// Stable tool subset: research, assistant, or coding. Narrows any --tool selection.
    #[arg(long, global = true)]
    pub tool_profile: Option<mecha_core::tool::profile::ToolProfile>,

    /// Set by the trigger runner, never by a flag: the allowlist above came
    /// from a trigger file's `tools` line — durable, deliberate config. The
    /// subagent-skip notice stays quiet then, on the outbox warning's own
    /// reasoning: a warning that fires every scheduled morning on a
    /// deliberately narrowed run is how a real typo later gets ignored.
    #[arg(skip)]
    pub tools_from_trigger: bool,
    /// Set by the front-end that owns the run, never by a flag: the surface
    /// the session will be recorded as, so `setup::build` can match the
    /// learned-rules block against it and record what it matched
    /// (`RunConfig::rules_surface`). A front-end that sets none matches no
    /// surface-scoped rule, which is the fail-closed reading of unknown.
    /// The test override (`MECHA_SESSION_KIND`) marks the session record
    /// and never this: a smoke test or an `exp` trial matches the block the
    /// shipped binary renders, and records that it did.
    #[arg(skip)]
    pub surface: Option<mecha_core::session::SessionKind>,
    /// Set by the front-end that owns the run, never by a flag of its own:
    /// the goal the run was handed from a store the owner wrote — `tasks
    /// work` its task, a trigger run its trigger, `run --goal` the owner's
    /// pointer, a question continuation the asking run's recorded one — so
    /// `setup::build` can match the learned-rules block toward it and record
    /// what it matched (`RunConfig::rules_goal`). A front-end whose one
    /// block serves many runs (`serve`, the front door), or whose runs have
    /// no structural goal, sets none, which matches no goal-scoped rule.
    /// Never the conversation's anchor, which can name another goal once the
    /// block is rendered — a resumed hand-over's older task, a goal the owner
    /// confirms mid-run.
    #[arg(skip)]
    pub goal: Option<mecha_core::goal::GoalRef>,
    /// The run's posture, when the front-end knows better than
    /// `setup::posture_for` would guess from the surface — `tasks work` and
    /// a question resume are `delegated` whatever their approver.
    #[arg(skip)]
    pub run_posture: Option<mecha_core::closure::RunPosture>,

    /// Only carry these skills (repeatable). Names are matched exactly.
    ///
    /// Narrows what `[skills]` already selected; it cannot enable a skill the
    /// config withheld.
    #[arg(long = "skill", global = true)]
    pub skills: Vec<String>,

    /// Skip MCP servers entirely.
    #[arg(long, global = true)]
    pub no_mcp: bool,

    /// Skip these MCP servers by name (repeatable). `--no-mcp` skips all of
    /// them; this is for turning one off while the rest stay.
    #[arg(long = "no-mcp-server", global = true)]
    pub no_mcp_servers: Vec<String>,

    /// Turn off reasoning. Cheaper and faster, but noticeably worse on
    /// multi-step work.
    #[arg(long, global = true)]
    pub no_thinking: bool,

    /// Don't inject learned rules from ~/.mecha/learning into the system
    /// prompt.
    #[arg(long, global = true)]
    pub no_learned_rules: bool,

    /// Don't load skills from ~/.mecha/skills — no `skill` tool, and nothing
    /// about them in the system prompt.
    #[arg(long, global = true)]
    pub no_skills: bool,

    /// Don't load ~/.mecha/charter.toml into the system prompt.
    #[arg(long, global = true)]
    pub no_charter: bool,

    /// Don't offer the `compact` tool — the run still compacts at its
    /// threshold, the model just cannot ask for it early.
    ///
    /// Exists for `mecha eval`, which forces it: the tool is registered from
    /// whether *this machine's* config gives the run a compaction threshold,
    /// so leaving it on would make the tool list — the front of the cached
    /// prefix — depend on local settings, and two scorecards stop being
    /// comparable for a reason neither of them records.
    #[arg(long, global = true)]
    pub no_compact_tool: bool,

    /// Don't run configured [[hook]] commands.
    #[arg(long, global = true)]
    pub no_hooks: bool,

    /// Don't escalate an ambiguous completed step to a quarantined model
    /// call, even if `[agent] step_escalation` is set — an opt-out, since the
    /// config field is already off by default. Forced by `mecha eval` for the
    /// same reason `--no-compact-tool` is: a machine's local config must not
    /// change what a scorecard measures.
    #[arg(long, global = true)]
    pub no_step_escalation: bool,

    /// Do not execute declared plan checks.
    #[arg(long, global = true)]
    pub no_step_checks: bool,
    /// Disable goal and charter planning guidance.
    #[arg(long, global = true)]
    pub no_goal_guidance: bool,

    /// Load no `[[rule]]` approval rules. **Not a flag**: a `forbid` is the
    /// operator's standing word, and a switch that lifts it for one run is
    /// the silently-degrading-guard shape. Set only by `mecha eval`'s
    /// `force_reproducible`, because a scorecard must not vary with this
    /// machine's rules file — a `forbid` in `config.toml` would turn a case's
    /// `shell` call into `Blocked by policy:` here and not elsewhere. Two
    /// doors reach it: that function, and `setup::switch_off` for
    /// `Lever::ApprovalRules` — which `Lever::bare` deliberately never
    /// throws, so a preset cannot lift the rules; only eval's own explicit
    /// line does.
    #[arg(skip)]
    pub no_rules: bool,

    /// Don't issue the in-run boredom notice, even if `[agent] boredom` is
    /// set — an opt-out for one of the two config switches that ship on
    /// (`compact_validate` is the other). Forced by
    /// `mecha eval`: the notice is a sentence in the model's context that
    /// this machine's config decides, and a scorecard must not depend on it.
    #[arg(long, global = true)]
    pub no_boredom: bool,

    /// Don't check a compaction summary for omissions, even if
    /// `[agent] compact_validate` is set. Forced by `mecha eval` for the
    /// same reason as `--no-boredom`: the check is a second model call that
    /// this machine's config turns on.
    #[arg(long, global = true)]
    pub no_compact_validate: bool,

    /// Compact on the reported size only, never on the forecast of the next
    /// request, even if `[agent] predictive_compaction` is set. The
    /// threshold stays. An experiment's lever; forced off by `mecha eval`
    /// with the rest of the set.
    #[arg(long, global = true)]
    pub no_predictive_compaction: bool,

    /// Don't carry a tool's state (the plan) across a compaction, even if
    /// `[agent] carried_state` is set. An experiment's lever; forced off by
    /// `mecha eval` with the rest of the set.
    #[arg(long, global = true)]
    pub no_carried_state: bool,

    /// Don't deliver the situation brief into the run's first user turn,
    /// even if `[agent] situation_brief` is set. The brief is still
    /// assembled and recorded. An experiment's lever (it ships off);
    /// forced off by `mecha eval` with the rest of the set.
    #[arg(long, global = true)]
    pub no_situation_brief: bool,

    /// Serve no past appraisal through `goal_context`, whatever `[agent]
    /// past_appraisals` says (`Lever::PastAppraisals`).
    #[arg(long, global = true)]
    pub no_past_appraisals: bool,

    /// Serve no planning success example through `goal_context`, whatever
    /// `[agent] success_examples` says (`Lever::SuccessExamples`).
    #[arg(long, global = true)]
    pub no_success_examples: bool,

    /// Don't route any tools through the outbox — configured [outbox] tools
    /// execute directly under the usual gates instead of being staged.
    #[arg(long, global = true)]
    pub no_outbox: bool,

    /// No inter-agent messaging: no `message_send` tool, and nothing from
    /// the mailbox is delivered into this run.
    #[arg(long, global = true)]
    pub no_messages: bool,

    /// Never fall back to another provider — a configured `fallbacks` list is
    /// ignored, and a transient failure that survives its retries fails the
    /// run instead of being answered by a different model.
    #[arg(long, global = true)]
    pub no_fallback: bool,

    /// Summarise older turns once the prompt passes this many tokens. Roughly
    /// two thirds of the model's context window is a reasonable setting.
    #[arg(long, global = true)]
    pub compact_at: Option<u64>,

    /// Print tool calls, results, and token usage as they happen.
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,

    /// Not a flag: read `~/.mecha/config.toml` only, ignoring any `mecha.toml`
    /// in the working directory.
    ///
    /// Set by the trigger runner, which builds this struct itself rather than
    /// parsing it. A scheduled unattended run must not take its MCP servers,
    /// hooks or tool surface from whatever repository the daemon happens to
    /// have been started in — see [`mecha_core::trigger`].
    #[arg(skip)]
    pub global_config_only: bool,

    /// Not a flag: appended to the system prompt after the user's own
    /// (never replacing it), ahead of the date stamp. Set by front-ends
    /// that need a standing block inside the cached prefix — voice-serve's
    /// speakable-output block is the first (docs/VOICE-RESEARCH.md, D10).
    #[arg(skip)]
    pub system_extra: Option<String>,
}

/// Same shape as `chat`, so switching between them is muscle memory.
#[derive(clap::Args, Debug)]
pub struct TuiArgs {
    /// Continue a saved session by id or unique prefix.
    #[arg(long)]
    pub resume: Option<String>,

    /// Don't write a transcript.
    #[arg(long)]
    pub no_session: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run one task and print the answer.
    Run(commands::run::Args),

    /// Interactive session in the terminal.
    Chat(commands::chat::Args),

    /// Full-screen session. The input line stays live while the agent works,
    /// so a message typed mid-run steers it instead of waiting for it.
    Tui(TuiArgs),

    /// Serve the agent to the local voice worker: an OpenAI-compatible
    /// chat endpoint on 127.0.0.1, one conversation per voice session.
    VoiceServe(commands::voice_serve::Args),

    /// Run the same agent over a JSONL file of prompts.
    Batch(commands::batch::Args),

    /// Score a model on a case set. The model bake-off rig.
    Eval(commands::eval::Args),

    /// Mine recorded sessions for user interventions and turn each into a
    /// learned reflection.
    Reflect(commands::reflect::Args),

    /// Read, edit and refuse the lessons `reflect` mined, before `learn`
    /// consolidates them into rules.
    Reflections(commands::reflections::Args),

    /// Absorb unprocessed reflections into the learned rule set.
    Learn(commands::learn::Args),

    /// Is the learning loop improving anything? Correction rate over time,
    /// rule-set health, and every consolidation pass. No model, no network.
    LearningReport(commands::learning_report::Args),

    /// Summarise closed sessions into episodes staged to the knowledge graph.
    Distill(commands::distill::Args),

    /// Probe whether the learned rules change the answers at the recorded
    /// moments the user stepped in.
    Validate(commands::validate::Args),

    /// Review, edit, release, or reject staged outbound actions.
    Outbox(commands::outbox::Args),

    /// Messages between this machine's agents: send one, read the backlog,
    /// see who is running. Delivery happens at the recipient's next turn.
    Msg(commands::msg::Args),

    /// What runs have generated, and removing what is past. Every producer —
    /// a trigger, a chat — writes into its own directory, which is also the
    /// path jail its runs get.
    Work(commands::work::Args),

    /// Which optional parts of mecha are on, and how to turn on the rest.
    ///
    /// Read from the config and the disk only — no network, so a server that
    /// starts on demand is never woken to be asked. Exit 0 whatever it finds:
    /// an install with features off is not a broken one. A config file that
    /// does not parse is an error, not a list.
    Features(commands::features::Args),

    /// Read every store — no network, no model, no tokens — and report what
    /// is silently wrong: dead mail logins, stuck outbox drafts, stalled
    /// frontdoor requests, failing triggers, failed units. On a terminal it
    /// offers each remedy; piped, it only reports. Exit 1 when it found
    /// anything.
    Doctor(commands::doctor::Args),

    /// Serve the tailnet web surface: dashboard, chat, review, voice.
    ///
    /// Binds 127.0.0.1 only; `tailscale serve` is the door, and the injected
    /// Tailscale-User-Login header must match `[web] owner_login` on every
    /// request. Refuses to start with no owner configured.
    Serve(commands::serve::Args),

    /// Read the run corpus and propose one change to try.
    ///
    /// The stage between `doctor` saying something is wrong and
    /// `eval --ab-config` saying whether a fix helped. It proposes; it does
    /// not measure and does not apply, because a diagnosis is right about
    /// which step failed roughly one time in seven and the whole design is
    /// arranged so that being wrong costs one measurement.
    Diagnose(commands::diagnose::Args),

    /// The harness improving itself, on the record: `ruminate` diagnoses one
    /// change nightly, measures it by counterfactual replay of recent
    /// sessions, and auto-applies only a measured, holdout-confirmed config
    /// win — reversibly, through an override layer your own config always
    /// beats. Everything else stages here for review.
    Harness(commands::harness::Args),

    /// A designed comparison over a chosen set of runs: arms that vary the
    /// closed lever set, a control, a prediction per treatment arm, and one
    /// isolated home per arm. A peer of `eval`, never a flag on it.
    Exp(commands::exp::Args),

    /// Requests that arrived through the public surface, and the quarantine
    /// they pass through before any run with tools is told about them.
    /// `factory-publish drain` fetches them; this is what happens next.
    Frontdoor(commands::frontdoor::Args),

    /// Triage the inbox: classify, read, dismiss.
    Mail(commands::mail::Args),

    /// The GTD board in the knowledge graph: what is on it, capture, and
    /// moving a task through its lifecycle.
    ///
    /// The same board `/tasks` shows and the model reads through `kg_task_*`
    /// — one store, reached through the tool surface from every side.
    Tasks(commands::tasks::Args),
    /// Durable follow-through, commitments, completion checks and today’s priorities.
    Workflow(commands::workflow::Args),

    /// What a delegated run got stuck on, and answering it — which resumes
    /// the run that asked, with your answer as its next turn.
    Questions(commands::questions::Args),
    /// Meeting polls: where each stands, the pick card, and the timer's sweep.
    Polls(commands::polls::Args),

    /// The knowledge graph from the terminal: search it, read an entity,
    /// capture a note — through the same `kg_*` tool surface the model uses.
    Kg(commands::kg::Args),
    /// Two readers with different sources ask each other about one entity.
    Gossip(commands::gossip::GossipArgs),
    /// Judge whether queued generalisations hold beyond their one source.
    Corroborate(commands::corroborate::CorroborateArgs),
    /// Judge queued claims against the evidence they were extracted from.
    Vet(commands::vet::VetArgs),

    /// Prompts that run on a schedule: a morning briefing, overnight inbox
    /// triage, calendar prep. `tick` fires what is due; `daemon` loops it.
    Trigger(commands::trigger::Args),

    /// Driving mecha from Slack: the tokens, and who is allowed to drive.
    /// `auth` stores the credential, `link` binds an owner by a one-time code
    /// printed here — which proves shell access to this machine, where an
    /// email address proves only what the workspace claims about it.
    Slack(commands::slack::Args),

    /// Review, accept, or reject rule changes staged by `mecha learn --propose`.
    Proposals(commands::proposals::Args),

    /// Everything waiting on you, across every store — and the graph's
    /// merge queue, which nothing in mecha could reach before.
    Review(commands::review::Args),

    /// Rule tenure: ledger tallies per rule, retire/restore, and staging
    /// retirements for rules the validation ledger keeps convicting.
    Rules(commands::rules::Args),

    /// Re-run a recorded session against recorded tool results and report
    /// where the model diverged.
    Replay(commands::replay::Args),

    /// List the tools an agent would see.
    Tools(commands::tools::Args),

    /// List the skills an agent would carry — the procedures you have written
    /// for it in ~/.mecha/skills, and which of them this run would load.
    Skills(commands::skills::Args),

    /// Extract a PDF — its text layer, and a local OCR model's transcript for
    /// scans — and manage the extraction cache.
    Document(commands::document::Args),

    /// The image library — recurring characters and styles that
    /// image_generate's `cast` and `style` compile against. List, add,
    /// approve a model's proposal, lock for browsing.
    Imagelib(commands::imagelib::Args),

    /// Personas — characters you write and talk to, kept apart from the
    /// assistant. Make one, edit who they are, lock, group; relationship
    /// templates.
    Persona(commands::persona::Args),

    /// Show the standing priorities in ~/.mecha/charter.toml, ranked highest
    /// first. Only a person edits a charter — `mecha charter edit` hands the
    /// file to $EDITOR — and never a model.
    Charter(commands::charter::Args),

    /// Inspect saved transcripts.
    #[command(subcommand)]
    Sessions(commands::sessions::Args),

    /// What this install still needs, and the way to each.
    ///
    /// Reads the local server's own `/props` rather than trusting config, so
    /// `context_window`, `vision` and `model` come off the wire instead of
    /// being typed — which is the class of mistake nothing can detect later.
    Setup(commands::setup::Args),

    /// Show or create configuration.
    #[command(subcommand)]
    Config(commands::config::Args),

    /// The local model router: what it can serve, and which model it holds.
    /// Loading one is the pick — every default run follows it, with no
    /// restart and no setting to edit.
    Model(commands::model::Args),
}

impl Command {
    /// Whether a default provider in this command may follow the router's
    /// loaded model. Not `mecha eval`: a scorecard grades the model it names,
    /// and two taken a week apart must not be different models under one
    /// condition. It is still observed, for its permit seats (found on review).
    fn may_follow(&self) -> bool {
        !matches!(self, Command::Eval(_))
    }

    /// Whether this command is **one run** for its whole life, and so holds
    /// the router until it returns (D13): a switch waits for it, and it waits
    /// for a switch before it starts. The long-lived ones — which serve many
    /// runs, and hold per turn or per fire themselves — are `false`, or a
    /// switch would wait for a daemon forever; so are the readers that build
    /// a registry without running a model.
    ///
    /// **Exhaustive, no wildcard**, like [`runs_a_model`](Self::runs_a_model):
    /// a new subcommand decides. Consulted only where that one is `true`.
    ///
    /// **A command listed here must not wait on a `mecha` child that is also
    /// listed here** (against the same `MECHA_HOME`): with a switch pending,
    /// the child yields to the switch, the switch waits for the parent, and
    /// the parent waits for the child — deadlocked until "switch now". That is
    /// why `workflow` is `false` (its `resume` waits on `tasks work`), and why
    /// `exp`'s trials are safe (their own `MECHA_HOME`). A child holding on
    /// its own is otherwise right: an inherited "covered by the parent" mark
    /// reached detached `session_end` hooks too, which outlive the parent
    /// and would have run unheld (review of #350).
    ///
    /// **The rule is every holder's, not only these commands'.** A trigger
    /// fire and a web, voice or Slack turn hold for their whole run, and their
    /// agent can call `shell: mecha run …` (or `tasks work`, `distill`, …): with
    /// a switch pending, the child waits for the switch, the switch for the
    /// turn, the turn for its shell call. Nothing breaks the cycle but `mecha
    /// model use --now` or `mecha model cancel-switch`, and `model use` names
    /// the holder it waits on (`trigger <name>`, `web chat`) — the place to
    /// look (review of #350).
    fn is_one_run(&self) -> bool {
        use commands::{exp, frontdoor, harness, mail, questions, sessions, tasks};
        match self {
            Command::Run(_)
            | Command::Batch(_)
            | Command::Eval(_)
            | Command::Reflect(_)
            | Command::Learn(_)
            | Command::Distill(_)
            | Command::Validate(_)
            | Command::Diagnose(_)
            | Command::Gossip(_)
            | Command::Corroborate(_)
            | Command::Vet(_)
            | Command::Replay(_) => true,
            // **Per subcommand where only some run a model** (review of D13):
            // held per command, `tasks stop` — the documented way to stop a
            // detached `tasks work` — waited behind a switch that was waiting
            // for that same `tasks work`, and serve's board and mail pages
            // (`tasks list --json`, `mail recent --json`) timed out for as
            // long as a switch was pending.
            Command::Tasks(a) => matches!(a.cmd, Some(tasks::Cmd::Work { .. })),
            Command::Mail(a) => matches!(
                a.cmd,
                Some(
                    mail::Cmd::Classify { .. }
                        | mail::Cmd::Eval { .. }
                        | mail::Cmd::Reflect { .. }
                        | mail::Cmd::Reply { .. }
                        | mail::Cmd::Forward { .. }
                        | mail::Cmd::Schedule { .. }
                )
            ),
            Command::Frontdoor(a) => matches!(
                a.cmd,
                Some(frontdoor::Cmd::Extract { .. } | frontdoor::Cmd::Triage { .. })
            ),
            Command::Questions(a) => matches!(a.cmd, Some(questions::Cmd::Answer { .. })),
            Command::Harness(a) => matches!(a.cmd, harness::Cmd::Ruminate { .. }),
            Command::Exp(a) => matches!(a.cmd, exp::Cmd::Run { .. } | exp::Cmd::Judge { .. }),
            // `appraise --probe` and `compare` replay sessions against a model.
            Command::Sessions(a) => matches!(
                a,
                sessions::Args::Appraise { probe: true, .. } | sessions::Args::Compare { .. }
            ),
            // `persona memory write` runs the memory writer on the local
            // model, so it holds and follows like `distill`; every other
            // `persona` verb reads or edits the store and runs no model.
            Command::Persona(a) => matches!(
                a.cmd,
                commands::persona::Cmd::Memory {
                    cmd: commands::persona::MemoryCmd::Write { .. }
                }
            ),
            // `workflow resume` starts `mecha tasks work` as a child, which
            // holds for itself; held here too, `--now` signalled the parent
            // and left the child running unheld (review of D13).
            Command::Workflow(_) => false,
            // Long-lived: hold per turn (`follow::Follower::enter`) or per
            // fire (`trigger::run_agent`), or do not follow yet (`chat`,
            // `tui` — REMOTE-SURFACE-DESIGN §14 step 4).
            Command::Chat(_)
            | Command::Tui(_)
            | Command::VoiceServe(_)
            | Command::Serve(_)
            | Command::Slack(_)
            | Command::Trigger(_)
            // Run no model: configuration, rules, or a registry built to list.
            | Command::Setup(_)
            | Command::Rules(_)
            | Command::Outbox(_)
            | Command::Kg(_)
            | Command::Tools(_)
            | Command::Reflections(_)
            | Command::LearningReport(_)
            | Command::Msg(_)
            | Command::Work(_)
            | Command::Doctor(_)
            | Command::Features(_)
            | Command::Polls(_)
            | Command::Proposals(_)
            | Command::Review(_)
            | Command::Skills(_)
            | Command::Imagelib(_)
            | Command::Document(_)
            | Command::Charter(_)
            | Command::Config(_)
            | Command::Model(_) => false,
        }
    }

    /// Whether this command prints the features upgrade notice
    /// (`commands::features::print_notices`): the starts of a session or a
    /// long-running service, where the owner or a unit's journal reads it
    /// once. Not the one-shot verbs `mecha serve` runs as children per
    /// request, whose stderr would repeat it into every log line. `batch`
    /// is a run started by the owner, many times over, on the same gated
    /// registry `run` builds — nothing spawns it per request — so a batch over
    /// an install from before `[features]` is told why its tools are missing
    /// (the #445 leftover).
    fn announces_features(&self) -> bool {
        matches!(
            self,
            Command::Run(_) | Command::Chat(_) | Command::Tui(_) | Command::Serve(_)
        ) || matches!(self, Command::VoiceServe(_) | Command::Batch(_))
            || matches!(self, Command::Slack(a) if matches!(a.cmd, Some(commands::slack::Cmd::Connect)))
            || matches!(self, Command::Trigger(a) if matches!(a.cmd, Some(commands::trigger::Cmd::Daemon { .. })))
    }

    /// Whether this command may resolve a default provider — run a model, or
    /// build an agent — and so needs [`follow_the_loaded_model`]'s snapshot.
    ///
    /// **Exhaustive, no wildcard, on purpose:** a new subcommand has to
    /// decide. The two mistakes cost differently. A model-running command
    /// wrongly listed `false` names the default model and silently swaps the
    /// owner's pick back out; an observer wrongly listed `true` pays a
    /// loopback round trip. So unsure is `true` — except where the command
    /// promises no network, which `mecha doctor`'s module doc does (found on
    /// review).
    fn runs_a_model(&self) -> bool {
        match self {
            Command::Run(_)
            | Command::Chat(_)
            | Command::Tui(_)
            | Command::VoiceServe(_)
            | Command::Batch(_)
            | Command::Eval(_)
            | Command::Reflect(_)
            | Command::Learn(_)
            | Command::Distill(_)
            | Command::Validate(_)
            | Command::Setup(_)
            | Command::Serve(_)
            | Command::Diagnose(_)
            | Command::Harness(_)
            | Command::Exp(_)
            | Command::Frontdoor(_)
            | Command::Mail(_)
            | Command::Tasks(_)
            | Command::Workflow(_)
            | Command::Questions(_)
            | Command::Gossip(_)
            | Command::Corroborate(_)
            | Command::Vet(_)
            | Command::Slack(_)
            | Command::Trigger(_)
            | Command::Replay(_)
            // These build the tool registry (`prepare_tools`), whose output
            // budget is the default provider's window; `sessions` also has a
            // subcommand that builds an agent.
            | Command::Outbox(_)
            | Command::Kg(_)
            | Command::Tools(_)
            | Command::Sessions(_)
            // `rules propose-retirements` counts the ledger rows of the model
            // in use, so it resolves the default provider as `validate` does.
            | Command::Rules(_) => true,
            // `persona memory write` runs the memory writer on the local
            // model, so it holds and follows like `distill`; every other
            // `persona` verb reads or edits the store and runs no model.
            Command::Persona(a) => matches!(
                a.cmd,
                commands::persona::Cmd::Memory {
                    cmd: commands::persona::MemoryCmd::Write { .. }
                }
            ),
            // Readers of stores, and `mecha model`, which asks the router
            // directly rather than through a snapshot.
            Command::Reflections(_)
            | Command::LearningReport(_)
            | Command::Msg(_)
            | Command::Work(_)
            | Command::Doctor(_)
            | Command::Features(_)
            | Command::Polls(_)
            | Command::Proposals(_)
            | Command::Review(_)
            | Command::Skills(_)
            | Command::Imagelib(_)
            | Command::Document(_)
            | Command::Charter(_)
            | Command::Config(_)
            | Command::Model(_) => false,
        }
    }
}

/// This process's hold on the router (D13), kept where an early exit reaches
/// it. It lived in `dispatch`'s frame, and `std::process::exit` skips
/// destructors: every command that ends non-zero — a failed eval case, a
/// failed batch item, a refused `mecha run` — left its hold file behind. A
/// dead pid's hold is swept, never waited on, so nothing blocked; the files
/// just accumulated (five `mecha eval` holds on 2026-09-30).
static RUN_HOLD: std::sync::Mutex<Option<mecha_core::hold::Held>> = std::sync::Mutex::new(None);

/// Release this process's router hold, if it has one.
fn release_run_hold() {
    let held = RUN_HOLD.lock().unwrap_or_else(|e| e.into_inner()).take();
    drop(held);
}

/// End the process with `code`, releasing its router hold first. Every
/// command's non-zero exit goes through here rather than
/// `std::process::exit`, which would skip the hold's drop
/// (`no_command_exits_around_the_hold` keeps it so).
pub(crate) fn exit_with(code: i32) -> ! {
    release_run_hold();
    std::process::exit(code)
}

/// Releases the run hold when `dispatch` returns, as the local it replaced
/// did when it went out of scope.
struct ReleaseRunHold;

impl Drop for ReleaseRunHold {
    fn drop(&mut self) {
        release_run_hold();
    }
}

/// This process's hold on the router (D13), for a command that is one run.
///
/// "Switch now" stops it, and the children it covers (`follow::cover_child`),
/// as Ctrl-C would stop a foreground job: the signal a run started through
/// `interrupt::run_interruptible` turns into a cancel at its next safe point,
/// keeping the partial answer. A command that does not catch it ends — which
/// is what the owner asked for by not waiting.
async fn hold_for_this_run(global: &GlobalOpts) -> Result<Option<mecha_core::hold::Held>> {
    let Ok(cfg) = load_config(global) else {
        // The command reports its own config error.
        return Ok(None);
    };
    // The subcommand's name and nothing after it: `mecha run "<prompt>"`
    // must not leave the prompt in a file under ~/.mecha/holds. Only a word
    // clap itself knows as a subcommand is taken, so neither a flag's value
    // (`--provider local`) nor a word of the prompt can be the label.
    let sub = subcommand_label(std::env::args().skip(1));
    let held =
        crate::follow::hold_router(&cfg, global.provider.as_deref(), &format!("mecha {sub}"))
            .await?;
    if let Some(h) = &held {
        h.on_cancel(|| {
            // First, no further model request leaves this process: the
            // interrupt stops the run in flight, but a loop that runs one
            // agent per item (`frontdoor triage`, mail drafting) would start
            // the next on its startup binding and load the old model back
            // (review of D13).
            mecha_core::provider::halt("the model was switched with `mecha model use --now`");
            // Then the children this hold covers (`exp run`'s trials), which
            // the signal below would not reach.
            crate::follow::interrupt_covered_children();
            // SAFETY: signalling this process; no memory is touched.
            unsafe {
                libc::kill(libc::getpid(), libc::SIGINT);
            }
        });
    }
    Ok(held)
}

/// The first argument clap itself knows as a subcommand — never a flag's value
/// or a word of the prompt, so it is safe to leave in `~/.mecha/holds`.
fn subcommand_label(args: impl Iterator<Item = String>) -> String {
    use clap::CommandFactory;
    let cmd = Cli::command();
    let names: Vec<&str> = cmd.get_subcommands().map(|c| c.get_name()).collect();
    args.into_iter()
        .find(|a| names.contains(&a.as_str()))
        .unwrap_or_default()
}

fn load_config(global: &GlobalOpts) -> Result<mecha_core::config::Config> {
    if global.global_config_only {
        mecha_core::config::Config::load_global()
    } else {
        std::env::current_dir()
            .map_err(anyhow::Error::from)
            .and_then(|cwd| mecha_core::config::Config::load(&cwd))
    }
}

/// Which model the llama-server router has loaded, snapshotted once for this
/// process so every default provider in it follows the owner's pick
/// (`provider::router`, REMOTE-SURFACE-DESIGN §14). Best-effort by design: a
/// config that does not load is the command's own error to report, and a
/// router that is down leaves the default standing — one loopback round trip
/// when it is up, nothing when nothing listens.
async fn follow_the_loaded_model(global: &GlobalOpts, may_follow: bool) {
    let Ok(cfg) = load_config(global) else { return };
    // A process given `--model` or `--provider` has named what it runs, so it
    // does not follow — the passes that resolve `cfg.provider(global.provider)`
    // themselves (lesson, pointwise, gossip, …) never reach `setup`'s pin. It
    // is still observed: its permit pool is sized to what is loaded.
    let follows = may_follow && global.model.is_none() && global.provider.is_none();
    for warning in mecha_core::provider::router::observe(&cfg, follows).await {
        tracing::warn!("{warning}");
    }
}

#[tokio::main]
async fn main() {
    // Quiet by default; `MECHA_LOG=debug` turns on the internals.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("MECHA_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        // Not `std::io::stderr` directly: under `mecha tui` stderr *is* the
        // alternate screen, and a warning written to it scribbles through the
        // frame and stays there — ratatui repaints by diffing its own buffer,
        // so it never repaints cells it did not write. `logs` holds the lines
        // instead, but only once a front-end says it has taken the screen.
        .with_writer(logs::Make)
        .without_time()
        .init();

    if let Err(e) = dispatch().await {
        eprintln!("mecha: {e:#}");
        std::process::exit(1);
    }
}

async fn dispatch() -> Result<()> {
    let cli = Cli::parse();
    // D13: a command that is one run holds the router for its whole life,
    // taken before the snapshot below so it can never resolve the model a
    // pending switch is replacing. Released when the command returns, or by
    // `exit_with` when it ends early.
    let _release = ReleaseRunHold;
    if cli.command.runs_a_model() && cli.command.is_one_run() {
        let held = hold_for_this_run(&cli.global).await?;
        *RUN_HOLD.lock().unwrap_or_else(|e| e.into_inner()) = held;
    }
    if cli.command.runs_a_model() {
        follow_the_loaded_model(&cli.global, cli.command.may_follow()).await;
    }
    if cli.command.announces_features() {
        commands::features::print_notices();
    }
    match cli.command {
        Command::Run(args) => commands::run::execute(&cli.global, args).await,
        Command::Chat(args) => commands::chat::execute(&cli.global, args).await,
        Command::Tui(args) => tui::execute(&cli.global, args.resume, args.no_session).await,
        Command::VoiceServe(args) => commands::voice_serve::execute(&cli.global, args).await,
        Command::Batch(args) => commands::batch::execute(&cli.global, args).await,
        Command::Eval(args) => commands::eval::execute(&cli.global, args).await,
        Command::Reflect(args) => commands::reflect::execute(&cli.global, args).await,
        Command::Reflections(args) => commands::reflections::execute(args).await,
        Command::Learn(args) => commands::learn::execute(&cli.global, args).await,
        Command::Distill(args) => commands::distill::execute(&cli.global, args).await,
        Command::LearningReport(args) => commands::learning_report::execute(args).await,
        Command::Validate(args) => commands::validate::execute(&cli.global, args).await,
        Command::Outbox(args) => commands::outbox::execute(&cli.global, args).await,
        Command::Msg(args) => commands::msg::execute(args).await,
        Command::Work(args) => commands::work::execute(args).await,
        Command::Setup(args) => commands::setup::execute(&cli.global, args).await,
        Command::Doctor(args) => commands::doctor::execute(args).await,
        Command::Features(args) => commands::features::execute(args).await,
        Command::Serve(args) => commands::serve::execute(args).await,
        Command::Diagnose(args) => commands::diagnose::execute(&cli.global, args).await,
        Command::Harness(args) => commands::harness::execute(&cli.global, args).await,
        Command::Exp(args) => commands::exp::execute(&cli.global, args).await,
        Command::Frontdoor(args) => commands::frontdoor::run(&cli.global, args).await,
        Command::Mail(args) => commands::mail::run(&cli.global, args).await,
        Command::Tasks(args) => commands::tasks::run(&cli.global, args).await,
        Command::Workflow(args) => commands::workflow::run(&cli.global, args).await,
        Command::Questions(args) => commands::questions::run(&cli.global, args).await,
        Command::Polls(args) => commands::polls::run(&cli.global, args).await,
        Command::Kg(args) => commands::kg::run(&cli.global, args).await,
        Command::Gossip(args) => commands::gossip::run(&cli.global, &args).await,
        Command::Corroborate(args) => commands::corroborate::run(&cli.global, &args).await,
        Command::Vet(args) => commands::vet::run(&cli.global, &args).await,
        Command::Slack(args) => commands::slack::run(&cli.global, args).await,
        Command::Trigger(args) => commands::trigger::execute(&cli.global, args).await,
        Command::Proposals(args) => commands::proposals::execute(args).await,
        Command::Review(args) => commands::review::execute(args).await,
        Command::Rules(args) => commands::rules::execute(&cli.global, args).await,
        Command::Replay(args) => commands::replay::execute(&cli.global, args).await,
        Command::Tools(args) => commands::tools::execute(&cli.global, args).await,
        Command::Skills(args) => commands::skills::execute(&cli.global, args).await,
        Command::Imagelib(args) => commands::imagelib::execute(&cli.global, args).await,
        Command::Document(args) => commands::document::execute(args).await,
        Command::Persona(args) => commands::persona::execute(&cli.global, args).await,
        Command::Charter(args) => commands::charter::execute(&cli.global, args).await,
        Command::Sessions(args) => commands::sessions::execute(&cli.global, args).await,
        Command::Config(args) => commands::config::execute(&cli.global, args).await,
        Command::Model(args) => commands::model::execute(&cli.global, args).await,
    }
}

#[cfg(test)]
mod tests {
    /// An early exit releases this process's router hold: the file a real
    /// hold wrote is gone once `release_run_hold` — what `exit_with` calls
    /// before `std::process::exit` — has run. Before, the hold lived in
    /// `dispatch`'s frame and a non-zero exit left the file behind.
    #[test]
    fn an_early_exit_releases_the_run_hold() {
        let dir = std::env::temp_dir().join(format!("mecha-run-hold-{}", std::process::id()));
        let holds = mecha_core::hold::Holds::new(&dir);
        let held = holds
            .try_hold("http://127.0.0.1:9", "mecha eval")
            .unwrap()
            .expect("no switch is pending in a fresh directory");
        assert_eq!(holds.live("http://127.0.0.1:9").len(), 1);
        *super::RUN_HOLD.lock().unwrap() = Some(held);
        super::release_run_hold();
        assert!(
            holds.live("http://127.0.0.1:9").is_empty(),
            "the hold outlived the exit"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No command ends the process around the hold: every non-zero exit in
    /// the commands goes through `exit_with`, which releases it first. A bare
    /// `std::process::exit` added later is how the leak comes back.
    #[test]
    fn no_command_exits_around_the_hold() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src.clone()];
        let mut found = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") && path != src.join("main.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (i, line) in text.lines().enumerate() {
                        let code = line.trim_start();
                        if !code.starts_with("//") && code.contains("std::process::exit(") {
                            found.push(format!("{}:{}", path.display(), i + 1));
                        }
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "exit through `crate::exit_with`: {found:?}"
        );
    }

    /// D13: a command holds the router for its life only where it runs a
    /// model. Held per command, `tasks stop` — the way to stop a detached
    /// `tasks work` — waited behind a switch waiting for that same run, and
    /// serve's board page (`tasks list`) timed out while a switch was pending.
    #[test]
    fn only_a_subcommand_that_runs_a_model_holds_the_router_for_its_life() {
        use clap::Parser;
        let one_run = |argv: &[&str]| {
            let cli = super::Cli::try_parse_from(argv).expect("parses");
            cli.command.runs_a_model() && cli.command.is_one_run()
        };
        for held in [
            &["mecha", "run", "hello"][..],
            &["mecha", "tasks", "work", "task-1a2b3c4d"],
            &["mecha", "mail", "classify"],
            &["mecha", "frontdoor", "triage"],
            &["mecha", "harness", "ruminate"],
            &["mecha", "sessions", "appraise", "--probe"],
        ] {
            assert!(one_run(held), "{held:?} runs a model and must hold");
        }
        for free in [
            &["mecha", "tasks", "list"][..],
            &["mecha", "tasks", "stop", "task-1a2b3c4d"],
            &["mecha", "mail", "recent"],
            &["mecha", "frontdoor", "list"],
            &["mecha", "workflow", "list"],
            &["mecha", "harness", "list"],
            &["mecha", "serve"],
            &["mecha", "sessions", "list"],
            &["mecha", "model", "list"],
        ] {
            assert!(
                !one_run(free),
                "{free:?} runs no model here and must not hold"
            );
        }
    }

    /// The hold's label is a subcommand name and nothing else: not a flag's
    /// value, not a word of the prompt.
    #[test]
    fn a_holds_label_is_never_user_content() {
        let label = |args: &[&str]| super::subcommand_label(args.iter().map(|a| a.to_string()));
        assert_eq!(
            label(&["--provider", "local", "run", "my secret plan"]),
            "run"
        );
        assert_eq!(label(&["tasks", "work", "task-1"]), "tasks");
        assert_eq!(label(&["--system", "classified", "run", "x"]), "run");
        assert_eq!(label(&["nothing-known"]), "");
    }

    use super::*;

    /// The router snapshot is taken where a default provider can be resolved,
    /// and never by `mecha doctor`, whose module doc promises no network.
    #[test]
    fn doctor_does_not_probe_the_router_and_a_run_does() {
        let cmd = |argv: &[&str]| Cli::try_parse_from(argv).unwrap().command;
        assert!(!cmd(&["mecha", "doctor"]).runs_a_model());
        assert!(!cmd(&["mecha", "model", "list"]).runs_a_model());
        assert!(cmd(&["mecha", "run", "hello"]).runs_a_model());
        assert!(cmd(&["mecha", "mail", "classify"]).runs_a_model());
        // A scorecard names its model: eval observes, and never follows.
        assert!(cmd(&["mecha", "eval", "cases.toml"]).runs_a_model());
        // Retirement counts the rows of the model in use, so it must see the
        // router's resident model (#346).
        assert!(cmd(&["mecha", "rules", "propose-retirements"]).runs_a_model());
        assert!(!cmd(&["mecha", "eval", "cases.toml"]).may_follow());
        assert!(cmd(&["mecha", "run", "hello"]).may_follow());
    }
}
