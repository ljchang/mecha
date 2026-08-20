//! `mecha` — an agent harness for local models.

mod approve;
mod commands;
mod editor;
mod interrupt;
mod probe;
mod render;
mod review_policy;
mod setup;
mod slack;
mod tui;

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

    /// Don't run configured [[hook]] commands.
    #[arg(long, global = true)]
    pub no_hooks: bool,

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

    /// Run the same agent over a JSONL file of prompts.
    Batch(commands::batch::Args),

    /// Score a model on a case set. The model bake-off rig.
    Eval(commands::eval::Args),

    /// Mine recorded sessions for user interventions and turn each into a
    /// learned reflection.
    Reflect(commands::reflect::Args),

    /// Absorb unprocessed reflections into the learned rule set.
    Learn(commands::learn::Args),

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

    /// Read every store — no network, no model, no tokens — and report what
    /// is silently wrong: dead mail logins, stuck outbox drafts, stalled
    /// frontdoor requests, failing triggers, failed units. On a terminal it
    /// offers each remedy; piped, it only reports. Exit 1 when it found
    /// anything.
    Doctor(commands::doctor::Args),

    /// Read the run corpus and propose one change to try.
    ///
    /// The stage between `doctor` saying something is wrong and
    /// `eval --ab-config` saying whether a fix helped. It proposes; it does
    /// not measure and does not apply, because a diagnosis is right about
    /// which step failed roughly one time in seven and the whole design is
    /// arranged so that being wrong costs one measurement.
    Diagnose(commands::diagnose::Args),

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

    /// Inspect saved transcripts.
    #[command(subcommand)]
    Sessions(commands::sessions::Args),

    /// Show or create configuration.
    #[command(subcommand)]
    Config(commands::config::Args),
}

#[tokio::main]
async fn main() {
    // Quiet by default; `MECHA_LOG=debug` turns on the internals.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("MECHA_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .without_time()
        .init();

    if let Err(e) = dispatch().await {
        eprintln!("mecha: {e:#}");
        std::process::exit(1);
    }
}

async fn dispatch() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => commands::run::execute(&cli.global, args).await,
        Command::Chat(args) => commands::chat::execute(&cli.global, args).await,
        Command::Tui(args) => tui::execute(&cli.global, args.resume, args.no_session).await,
        Command::Batch(args) => commands::batch::execute(&cli.global, args).await,
        Command::Eval(args) => commands::eval::execute(&cli.global, args).await,
        Command::Reflect(args) => commands::reflect::execute(&cli.global, args).await,
        Command::Learn(args) => commands::learn::execute(&cli.global, args).await,
        Command::Distill(args) => commands::distill::execute(&cli.global, args).await,
        Command::Validate(args) => commands::validate::execute(&cli.global, args).await,
        Command::Outbox(args) => commands::outbox::execute(&cli.global, args).await,
        Command::Msg(args) => commands::msg::execute(args).await,
        Command::Work(args) => commands::work::execute(args).await,
        Command::Doctor(args) => commands::doctor::execute(args).await,
        Command::Diagnose(args) => commands::diagnose::execute(&cli.global, args).await,
        Command::Frontdoor(args) => commands::frontdoor::run(&cli.global, args).await,
        Command::Mail(args) => commands::mail::run(&cli.global, args).await,
        Command::Tasks(args) => commands::tasks::run(&cli.global, args).await,
        Command::Gossip(args) => commands::gossip::run(&cli.global, &args).await,
        Command::Corroborate(args) => commands::corroborate::run(&cli.global, &args).await,
        Command::Vet(args) => commands::vet::run(&cli.global, &args).await,
        Command::Slack(args) => commands::slack::run(&cli.global, args).await,
        Command::Trigger(args) => commands::trigger::execute(&cli.global, args).await,
        Command::Proposals(args) => commands::proposals::execute(args).await,
        Command::Rules(args) => commands::rules::execute(args).await,
        Command::Replay(args) => commands::replay::execute(&cli.global, args).await,
        Command::Tools(args) => commands::tools::execute(&cli.global, args).await,
        Command::Skills(args) => commands::skills::execute(&cli.global, args).await,
        Command::Sessions(args) => commands::sessions::execute(&cli.global, args).await,
        Command::Config(args) => commands::config::execute(&cli.global, args).await,
    }
}
