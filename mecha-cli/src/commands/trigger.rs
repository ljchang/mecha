//! `mecha trigger` — prompts that run on a schedule.
//!
//! The shape here is deliberate: **`tick` is the primitive and `daemon` is a
//! loop over it.** Everything the daemon can do, a crontab line or a systemd
//! timer can do, because the daemon holds no state the store does not — being
//! due is a function of the ledger and the clock, so a scheduler that was
//! asleep, restarted, or never started at all reaches the same answer as one
//! that has been running all week. That is what makes `tick --dry-run` an
//! honest preview rather than a second implementation of the schedule.
//!
//! What a fire actually is: a fresh `Conversation` (so taint never carries from
//! yesterday's run), a fresh agent built from the *global* config only, the
//! trigger's own budgets and permission mode, a recorded session, and a ledger
//! row. Nothing here reaches into the agent loop — an unattended run is the
//! same run as an interactive one, minus the human.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use mecha_core::agent::{Conversation, RunContext};
use mecha_core::config::PermissionMode;
use mecha_core::message::Message;
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use mecha_core::trigger::{CatchUp, Due, RunRecord, RunStatus, Trigger, TriggerStore};
use std::io::Write;
use tokio_util::sync::CancellationToken;

use crate::{setup, GlobalOpts};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

// `Add` is much larger than the rest, and clap cannot build a subcommand from a
// boxed field — the indirection the lint asks for is not available here. The
// enum is constructed once per process.
#[allow(clippy::large_enum_variant)]
#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// List triggers, when each next fires, and how the last run went (default).
    List,

    /// Add a trigger. Writes `~/.mecha/triggers/<name>.toml`, which you can
    /// then edit by hand.
    Add(AddArgs),

    /// Show one trigger's settings and its recent runs.
    Show {
        name: String,
        /// Print the last run's answer, read back from its session transcript.
        #[arg(long)]
        last: bool,
    },

    /// Open a trigger's file in $EDITOR.
    Edit { name: String },

    /// Delete a trigger. Its ledger rows stay as the record.
    Rm { name: String },

    /// Let a trigger fire again. Slots missed while it was off are subject to
    /// `catch_up` like any others — under the default (`always`) the most
    /// recent one fires at the next tick, so re-enabling a 07:00 briefing at
    /// noon delivers this morning's.
    Enable { name: String },
    /// Stop a trigger firing without deleting it or losing its history.
    Disable { name: String },

    /// Upcoming fire times, without running anything.
    Next {
        /// Only this trigger.
        name: Option<String>,
        #[arg(long, short = 'n', default_value_t = 5)]
        count: usize,
    },

    /// Run one trigger now, whatever its schedule says.
    ///
    /// Recorded with no slot, so it does not count as the scheduled fire — a
    /// test run at noon must not cancel tomorrow's 07:00.
    Run { name: String },

    /// Fire whatever is due and exit. The primitive: drive it from cron, a
    /// systemd timer, or `mecha trigger daemon`.
    Tick {
        /// Say what would fire, and fire nothing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Tick once a minute until stopped.
    Daemon {
        /// Print a systemd user unit for this daemon and exit, running
        /// nothing.
        ///
        /// Exists because the alternative instruction — "copy
        /// `scripts/mecha-triggers.service`" — cannot be followed by anyone
        /// who installed from crates.io: the crate ships no `scripts/`
        /// directory, and a documented step that silently does not apply to
        /// the install path the docs lead with is worse than no step. The
        /// unit is printed rather than installed, because a scheduler is
        /// something to read before you let it run unattended.
        #[arg(long)]
        print_unit: bool,
    },

    /// Stop the run in flight, if there is one.
    ///
    /// Stops it at its next safe point, keeping the partial answer and the
    /// ledger row — the same path as Ctrl-C and the timeout. Not a signal: the
    /// run may belong to the daemon's process, and SIGTERM there would stop
    /// the whole scheduler.
    Cancel { name: String },

    /// The run ledger, newest first.
    Runs {
        name: Option<String>,
        #[arg(long, short = 'n', default_value_t = 20)]
        count: usize,
    },
}

/// What `add` takes *beyond* the global flags.
///
/// Everything that shapes a run — `--provider`, `--model`, `--workspace`,
/// `--tool`, `--no-mcp`, `--max-turns`, `--max-output-tokens`, `--max-cost`,
/// `--yes`/`--read-only` — is already a global flag, and `add` records exactly
/// those into the trigger file. So the flags that describe a run are the same
/// whether you run it now or every morning, and there is one vocabulary to
/// learn rather than two that can drift apart.
#[derive(clap::Args, Debug)]
pub struct AddArgs {
    /// Lowercase letters, digits, `-` and `_`. It is the filename.
    pub name: String,

    /// Five-field cron — `minute hour day-of-month month day-of-week` — or
    /// `@daily`/`@hourly`/`@weekly`. Seconds are not a field: `0 7 * * *` is 7am.
    // Long-only, and a plain comment so this does not reach `--help`: every
    // obvious short letter is already a global flag, and a clap collision is a
    // runtime panic rather than a compile error. See
    // `the_cli_has_no_conflicting_flags`.
    #[arg(long)]
    pub schedule: String,

    /// What to ask. `@path` reads it from a file.
    #[arg(long)]
    pub prompt: String,

    /// One line, shown under the trigger in `mecha trigger list`.
    #[arg(long)]
    pub description: Option<String>,

    /// IANA name. Defaults to `[agent] timezone`, and is written into the file
    /// either way so the schedule can never change meaning under it.
    #[arg(long)]
    pub timezone: Option<String>,

    /// Wall-clock ceiling on one run (`20m` by default).
    #[arg(long)]
    pub timeout: Option<String>,
    /// `always` (default), `never`, or a duration like `2h`: how late a missed
    /// slot may be and still run.
    #[arg(long)]
    pub catch_up: Option<String>,
    /// Command run with the answer on stdin.
    #[arg(long)]
    pub notify: Option<String>,
    /// Create it switched off.
    #[arg(long)]
    pub disabled: bool,
    /// A skill this run may load (repeatable).
    ///
    /// Named explicitly because an unattended run has nobody to ask: without
    /// this the effective instruction set of a scheduled run would grow every
    /// time an unrelated skill was written. Omit for a trigger that needs
    /// none, which is most of them.
    #[arg(long = "skill")]
    pub skills: Vec<String>,

    /// Overwrite an existing trigger of the same name.
    #[arg(long)]
    pub force: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd.unwrap_or(Cmd::List) {
        Cmd::List => list(),
        Cmd::Add(a) => add(global, a),
        Cmd::Show { name, last } => show(&name, last),
        Cmd::Edit { name } => edit(&name),
        Cmd::Rm { name } => {
            TriggerStore::open_default()?.remove(&name)?;
            println!("removed trigger `{name}` (its ledger rows stay as the record)");
            Ok(())
        }
        Cmd::Enable { name } => set_enabled(&name, true),
        Cmd::Disable { name } => set_enabled(&name, false),
        Cmd::Next { name, count } => next(name.as_deref(), count),
        Cmd::Run { name } => run_one(global, &name).await,
        Cmd::Tick { dry_run } => {
            tick(global, dry_run, None).await?;
            Ok(())
        }
        Cmd::Daemon { print_unit } => {
            if print_unit {
                print!("{}", systemd_unit()?);
                return Ok(());
            }
            daemon(global).await
        }
        Cmd::Cancel { name } => cancel(&name),
        Cmd::Runs { name, count } => runs(name.as_deref(), count),
    }
}

fn cancel(name: &str) -> Result<()> {
    let store = open()?;
    // Confirm the trigger exists before reporting on a run of it: `cancel
    // brifing` should say the name is wrong, not that nothing is running.
    let _ = store.get(name)?;
    if store.request_cancel(name)? {
        println!("asked `{name}` to stop — it will end at its next safe point");
    } else {
        println!("`{name}` is not running");
    }
    Ok(())
}

/// The zone a schedule is read in when the trigger does not name one.
fn config_tz() -> Option<Tz> {
    mecha_core::config::Config::load_global()
        .ok()
        .and_then(|c| c.agent.timezone())
}

fn open() -> Result<TriggerStore> {
    TriggerStore::open_default()
}

// ---------------------------------------------------------------- inspection

fn list() -> Result<()> {
    let store = open()?;
    let (triggers, problems) = store.list()?;
    for p in &problems {
        eprintln!("mecha: unreadable trigger — {p}");
    }
    if triggers.is_empty() {
        println!(
            "no triggers. Add one:\n  \
             mecha trigger add briefing --schedule '0 7 * * 1-5' \\\n    \
             --prompt \"Summarise my inbox and today's calendar.\""
        );
        return Ok(());
    }

    let now = Utc::now();
    let tz = config_tz();
    let last_slots = store.last_slots()?;
    let last_runs = last_run_per_trigger(&store)?;

    for t in &triggers {
        let state = if t.enabled { "" } else { "  (disabled)" };
        let when = if !t.enabled {
            "—".to_string()
        } else {
            match t.due(last_slots.get(&t.name).copied(), now, tz) {
                Due::Now { .. } | Due::Stale { .. } => "due now".to_string(),
                Due::Not { next: Some(next) } => {
                    format!("in {} ({})", human_gap(next - now), local(next, t.tz(tz)))
                }
                Due::Not { next: None } | Due::Disabled => "never".to_string(),
            }
        };
        let last = last_runs
            .get(&t.name)
            .map(|r| {
                format!(
                    "  last {} {}",
                    r.status.as_str(),
                    human_gap(now - r.started_at) + " ago"
                )
            })
            .unwrap_or_default();
        println!(
            "{:<20} {:<16} {:<28}{}{}",
            t.name,
            t.schedule.source(),
            when,
            last,
            state
        );
        if let Some(d) = &t.description {
            println!("{:22}{d}", "");
        }
    }
    Ok(())
}

fn last_run_per_trigger(
    store: &TriggerStore,
) -> Result<std::collections::BTreeMap<String, RunRecord>> {
    let mut out = std::collections::BTreeMap::new();
    for run in store.runs()? {
        out.insert(run.trigger.clone(), run);
    }
    Ok(out)
}

fn show(name: &str, last: bool) -> Result<()> {
    let store = open()?;
    let t = store.get(name)?;
    let tz = t.tz(config_tz());
    let now = Utc::now();

    println!(
        "trigger {}{}",
        t.name,
        if t.enabled { "" } else { " (disabled)" }
    );
    if let Some(d) = &t.description {
        println!("  {d}");
    }
    println!("  schedule    {} [{}]", t.schedule.source(), tz);
    if let Some(next) = t.next_fire(now, config_tz()) {
        println!(
            "  next fire   {} (in {})",
            local(next, tz),
            human_gap(next - now)
        );
    }
    println!("  catch up    {}", t.catch_up);
    println!("  permission  {:?}", t.permission_mode);
    // Printed even when empty, on the same rule as the resolved workspace
    // below: "what does this run actually carry" must not be answered by a
    // line that is not there.
    println!(
        "  skills      {}",
        if t.skills.is_empty() {
            "none".to_string()
        } else {
            t.skills.join(", ")
        }
    );
    println!(
        "  timeout     {}",
        mecha_core::trigger::render_duration(t.timeout_duration())
    );
    if let Some(p) = &t.provider {
        println!("  provider    {p}");
    }
    if let Some(m) = &t.model {
        println!("  model       {m}");
    }
    // Always shown, including the default a run would resolve to: "where is
    // this jailed" is the question, and an omitted line answers it with
    // silence.
    match &t.workspace {
        Some(w) => println!("  workspace   {}", w.display()),
        None => println!(
            "  workspace   {} (default)",
            mecha_core::work::producer_dir(&t.name)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "unresolvable".into())
        ),
    }
    if !t.tools.is_empty() {
        println!("  tools       {}", t.tools.join(", "));
    }
    if t.no_mcp {
        println!("  mcp         off");
    }
    if let Some(n) = t.max_turns {
        println!("  max turns   {n}");
    }
    if let Some(n) = t.max_output_tokens {
        println!("  max output  {n} tokens");
    }
    if let Some(c) = t.max_cost_usd {
        println!("  max cost    ${c:.2}");
    }
    if let Some(n) = &t.notify {
        println!("  notify      {n}");
    }
    println!("  file        {}", store.path_of(&t.name).display());
    println!("\nprompt:\n{}", indent(&t.prompt));

    let mine: Vec<RunRecord> = store
        .runs()?
        .into_iter()
        .filter(|r| r.trigger == t.name)
        .collect();
    if !mine.is_empty() {
        println!("\nrecent runs:");
        for r in mine.iter().rev().take(5) {
            print_run(r);
        }
    }

    if last {
        match mine.iter().rev().find(|r| r.session_id.is_some()) {
            Some(r) => print_answer(r)?,
            None => println!("\nno recorded run to read back"),
        }
    }
    Ok(())
}

/// The answer lives in the session transcript, which is the record — the ledger
/// keeps a summary and a pointer rather than a second copy that could disagree.
fn print_answer(run: &RunRecord) -> Result<()> {
    let id = run.session_id.as_deref().unwrap_or_default();
    let dir = Session::default_dir()?;
    let path = Session::find(&dir, id)?;
    let (_, convo) = Session::load(&path)?;
    let text = convo
        .messages
        .iter()
        .rev()
        .find(|m| m.role == mecha_core::Role::Assistant)
        .map(|m| m.text())
        .unwrap_or_default();
    println!(
        "\n── {} · session {id} ──\n{text}",
        run.started_at.to_rfc3339()
    );
    Ok(())
}

fn next(name: Option<&str>, count: usize) -> Result<()> {
    let store = open()?;
    let (triggers, _) = store.list()?;
    let tz = config_tz();
    for t in triggers.iter().filter(|t| name.is_none_or(|n| t.name == n)) {
        println!("{} [{}]", t.name, t.tz(tz));
        let mut at = Utc::now();
        for _ in 0..count {
            let Some(fire) = t.next_fire(at, tz) else {
                break;
            };
            println!(
                "  {}  (in {})",
                local(fire, t.tz(tz)),
                human_gap(fire - Utc::now())
            );
            at = fire;
        }
    }
    Ok(())
}

fn runs(name: Option<&str>, count: usize) -> Result<()> {
    let store = open()?;
    let all = store.runs()?;
    let mine: Vec<&RunRecord> = all
        .iter()
        .filter(|r| name.is_none_or(|n| r.trigger == n))
        .rev()
        .take(count)
        .collect();
    if mine.is_empty() {
        println!("no runs recorded yet");
        return Ok(());
    }
    for r in mine.into_iter().rev() {
        print_run(r);
    }
    Ok(())
}

fn print_run(r: &RunRecord) {
    let mut line = format!(
        "{}  {:<20} {:<18}",
        r.started_at.format("%Y-%m-%d %H:%M"),
        r.trigger,
        r.status.as_str()
    );
    if r.manual {
        line.push_str(" manual");
    }
    if r.turns > 0 {
        line.push_str(&format!(" {} turns", r.turns));
    }
    if r.staged > 0 {
        line.push_str(&format!(" · {} staged", r.staged));
    }
    if r.blocked_sends > 0 {
        line.push_str(&format!(" · {} blocked", r.blocked_sends));
    }
    if let Some(cause) = r.stop_cause {
        line.push_str(&format!(" · {}", cause.describe()));
    }
    if let Some(c) = r.cost_usd {
        line.push_str(&format!(" · ${c:.3}"));
    }
    if let Some(s) = &r.session_id {
        line.push_str(&format!(" · session {s}"));
    }
    println!("{line}");
    if let Some(e) = &r.error {
        println!("    error: {e}");
    } else if !r.summary.is_empty() {
        println!("    {}", r.summary);
    }
    // After the summary rather than instead of it: the run produced an answer,
    // and both facts matter — this one is the reason it did not arrive.
    if let Some(e) = &r.notify_error {
        println!("    notify: {e}");
    }
}

// ----------------------------------------------------------------- authoring

fn add(global: &GlobalOpts, a: AddArgs) -> Result<()> {
    Trigger::valid_name(&a.name)?;
    let store = open()?;
    anyhow::ensure!(
        a.force || !store.exists(&a.name),
        "trigger `{}` already exists (use --force to overwrite, or `mecha trigger edit {}`)",
        a.name,
        a.name
    );

    let schedule = a.schedule.parse()?;
    let prompt = setup::read_maybe_file(&a.prompt)?;
    let mut t = Trigger::new(&a.name, schedule, prompt);

    t.description = a.description;
    t.skills = a.skills.clone();
    // Resolve the zone *now* and write it down. Leaving it implicit would mean
    // a later edit to `[agent] timezone` silently moves every trigger, and "the
    // briefing arrived at the wrong time" is the hardest kind of bug to notice.
    t.timezone = Some(
        a.timezone
            .as_deref()
            .map(|n| n.parse::<Tz>().map(|tz| tz.to_string()))
            .transpose()
            .map_err(|_| anyhow::anyhow!("unknown timezone `{}`", a.timezone.unwrap_or_default()))?
            .or_else(|| config_tz().map(|tz| tz.to_string()))
            .unwrap_or_else(|| "UTC".to_string()),
    );
    // Read-only unless `--yes` was passed. `--yes` reads as "this scheduled run
    // may write and execute unattended", which is the decision being made, and
    // the confirmation below prints the resulting mode so it is never silent.
    t.permission_mode = if global.yes {
        PermissionMode::Allow
    } else {
        PermissionMode::ReadOnly
    };
    // Written down, not left implicit. An unset workspace fell through to the
    // daemon's working directory — `WorkingDirectory=%h` in the shipped unit —
    // which jails an unattended run over `~/.mecha/`. The trigger's own work
    // directory holds nothing sensitive, and being stable across runs is what
    // makes yesterday's output an ordinary file in today's run.
    t.workspace = match &global.workspace {
        Some(w) => Some(
            w.canonicalize()
                .with_context(|| format!("workspace {} does not exist", w.display()))?,
        ),
        None => Some(mecha_core::work::ensure(&a.name)?),
    };
    t.tools = global.tools.clone();
    t.no_mcp = global.no_mcp;
    t.max_turns = global.max_turns;
    t.max_output_tokens = global.max_output_tokens;
    t.max_cost_usd = global.max_cost;
    t.timeout = a.timeout;
    if let Some(c) = &a.catch_up {
        t.catch_up = c.parse::<CatchUp>()?;
    }
    t.notify = a.notify;
    t.enabled = !a.disabled;
    t.provider = global.provider.clone();
    t.model = global.model.clone();

    // A cost ceiling that cannot fire is worse than none: it reads as a
    // guarantee. Refuse at authoring time, where there is someone to tell.
    check_cost_cap(&t)?;

    store.save(&t)?;
    println!("wrote {}", store.path_of(&t.name).display());
    println!(
        "permission {} · timeout {} · catch up {}",
        match t.permission_mode {
            PermissionMode::Allow => "allow (this run may write and execute unattended)",
            PermissionMode::ReadOnly => "read-only (outbox drafts still stage)",
            PermissionMode::Ask => "ask — nothing is watching, so this denies writes",
        },
        mecha_core::trigger::render_duration(t.timeout_duration()),
        t.catch_up,
    );
    let tz = t.tz(config_tz());
    match t.next_fire(Utc::now(), config_tz()) {
        Some(next) => println!(
            "first fire {} (in {})",
            local(next, tz),
            human_gap(next - Utc::now())
        ),
        None => println!("warning: this schedule never fires"),
    }
    if t.enabled {
        println!("nothing runs it yet — start `mecha trigger daemon`, or point a timer at `mecha trigger tick`");
    }
    Ok(())
}

/// `--max-cost` on a provider with no prices silently never fires (the run
/// reports `cost_usd: None`). Interactive commands can live with that; a
/// trigger cannot, because the ceiling is the only thing standing between an
/// unattended loop and a bill.
fn check_cost_cap(t: &Trigger) -> Result<()> {
    let Some(cap) = t.max_cost_usd else {
        return Ok(());
    };
    let cfg = mecha_core::config::Config::load_global()?;
    let (name, provider) = cfg.provider(t.provider.as_deref())?;
    anyhow::ensure!(
        provider.pricing().is_some(),
        "trigger `{}` sets max_cost_usd = {cap}, but provider `{name}` has no \
         input_price_per_mtok/output_price_per_mtok configured — the cap would never \
         fire. Configure the prices, or drop the cap and bound the run with \
         max_turns/max_output_tokens instead.",
        t.name
    );
    Ok(())
}

fn edit(name: &str) -> Result<()> {
    let store = open()?;
    let path = store.path_of(name);
    anyhow::ensure!(path.exists(), "no trigger named `{name}`");
    let edited = crate::editor::edit_text(&std::fs::read_to_string(&path)?, "toml")?;
    // Parse before writing: a trigger file that does not load is a trigger that
    // does not fire, and the daemon would only say so at 03:00.
    let mut parsed: Trigger = toml::from_str(&edited).context("the edited file does not parse")?;
    parsed.name = name.to_string();
    store.save(&parsed)?;
    println!("saved {}", path.display());
    Ok(())
}

fn set_enabled(name: &str, enabled: bool) -> Result<()> {
    let store = open()?;
    let mut t = store.get(name)?;
    t.enabled = enabled;
    store.save(&t)?;
    println!("{} {}", if enabled { "enabled" } else { "disabled" }, name);
    Ok(())
}

// ------------------------------------------------------------------ the clock

/// Fire everything due. Returns how many ran.
async fn tick(
    global: &GlobalOpts,
    dry_run: bool,
    stop: Option<&CancellationToken>,
) -> Result<usize> {
    let store = open()?;
    let (triggers, problems) = store.list()?;
    for p in &problems {
        eprintln!("mecha: unreadable trigger — {p}");
    }
    let tz = config_tz();
    let now = Utc::now();
    let last_slots = store.last_slots()?;
    let mut fired = 0;

    for t in &triggers {
        match t.due(last_slots.get(&t.name).copied(), now, tz) {
            Due::Now { slot } => {
                if dry_run {
                    println!(
                        "{:<20} would fire for slot {}",
                        t.name,
                        local(slot, t.tz(tz))
                    );
                    continue;
                }
                // Claim first: the lock is what stops a slow run from stacking,
                // and it is held by the kernel so a crashed run frees it.
                let Some(_claim) = store.try_claim(&t.name)? else {
                    let mut rec = RunRecord::started(&t.name, Some(slot), false);
                    rec.status = RunStatus::SkippedOverlap;
                    rec.finished_at = Some(Utc::now());
                    rec.error = Some("a previous run of this trigger is still going".into());
                    store.append_run(&rec)?;
                    eprintln!(
                        "mecha: {} skipped — the previous run is still going",
                        t.name
                    );
                    continue;
                };
                fire(global, &store, t, Some(slot), false, stop).await?;
                fired += 1;
            }
            // Written down rather than silently dropped: "why did I not get my
            // briefing" should be answerable from the ledger.
            Due::Stale { slot, age } => {
                if dry_run {
                    println!(
                        "{:<20} would skip slot {} ({} old, past catch_up = {})",
                        t.name,
                        local(slot, t.tz(tz)),
                        human_gap(age),
                        t.catch_up
                    );
                    continue;
                }
                let mut rec = RunRecord::started(&t.name, Some(slot), false);
                rec.status = RunStatus::SkippedStale;
                rec.finished_at = Some(Utc::now());
                rec.error = Some(format!(
                    "missed by {}, past catch_up = {}",
                    human_gap(age),
                    t.catch_up
                ));
                store.append_run(&rec)?;
            }
            Due::Not { next } if dry_run => {
                let when = next
                    .map(|n| format!("in {}", human_gap(n - now)))
                    .unwrap_or_else(|| "never".into());
                println!("{:<20} not due ({when})", t.name);
            }
            Due::Not { .. } | Due::Disabled => {}
        }
    }
    Ok(fired)
}

/// Tick once a minute, on the minute, until interrupted.
///
/// One run at a time, and a long one delays the next tick rather than being
/// overlapped by it. That is the honest trade for an unattended scheduler
/// sharing one local model server: predictable load, and nothing lost, because
/// due-ness is computed from slots rather than from tick timing.
/// A systemd user unit naming *this* binary by absolute path.
///
/// `current_exe` rather than the string "mecha": the whole point is to be
/// runnable by someone whose install is not on the PATH systemd will see, and
/// a unit that resolves to a different binary than the one printing it is the
/// version-skew trap one layer down.
fn systemd_unit() -> anyhow::Result<String> {
    // Deliberately NOT `exe::self_exe()`: this path is written into a unit
    // file, where `/proc/self/exe` would name whatever process systemd
    // spawns — the one place the magic link is exactly wrong.
    let exe = std::env::current_exe()
        .context("resolving this binary's own path")?
        .display()
        .to_string();
    Ok(format!(
        "[Unit]\n\
         Description=mecha triggers: scheduled agent runs\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exe} trigger daemon\n\
         Restart=on-failure\n\
         RestartSec=30\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n\
         \n\
         # Install with:\n\
         #   systemctl --user daemon-reload\n\
         #   systemctl --user enable --now mecha-triggers\n\
         # And, so it survives logout:\n\
         #   loginctl enable-linger $USER\n"
    ))
}

async fn daemon(global: &GlobalOpts) -> Result<()> {
    let store = open()?;
    let (triggers, problems) = store.list()?;
    for p in &problems {
        eprintln!("mecha: unreadable trigger — {p}");
    }
    println!(
        "mecha trigger daemon · {} trigger(s), {} enabled · ticking every minute",
        triggers.len(),
        triggers.iter().filter(|t| t.enabled).count()
    );
    let _ = std::io::stdout().flush();

    let stop = CancellationToken::new();
    {
        let stop = stop.clone();
        tokio::spawn(async move {
            let mut term =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(s) => s,
                    Err(_) => return,
                };
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            eprintln!("\nstopping — any run in flight stops at its next safe point");
            stop.cancel();
        });
    }

    loop {
        if stop.is_cancelled() {
            return Ok(());
        }
        if let Err(e) = tick(global, false, Some(&stop)).await {
            // A tick that fails must not end the daemon: the usual cause is a
            // transient (an unreadable store, a full disk), and a scheduler that
            // exits on the first bad night stops being a scheduler.
            eprintln!("mecha: tick failed — {e:#}");
        }
        tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            _ = tokio::time::sleep(until_next_minute()) => {}
        }
    }
}

/// Sleep to the next wall-clock minute rather than for sixty seconds, so ticks
/// stay aligned with the schedule they are checking however long a run took.
fn until_next_minute() -> std::time::Duration {
    let now = Utc::now();
    let secs = 60 - (now.timestamp() % 60);
    std::time::Duration::from_secs(secs.clamp(1, 60) as u64)
}

async fn run_one(global: &GlobalOpts, name: &str) -> Result<()> {
    let store = open()?;
    let t = store.get(name)?;
    let Some(_claim) = store.try_claim(name)? else {
        anyhow::bail!("`{name}` is already running");
    };
    // No slot: a manual run is evidence, not a fire.
    fire(global, &store, &t, None, true, None).await
}

// -------------------------------------------------------------------- firing

/// What the model is told about the situation it is running in.
///
/// Three facts, each of which changes what a good answer looks like: nobody is
/// there, so a question is a dead end; anything outbound is a draft, so
/// reporting it as sent would be a lie; and the answer goes to a person reading
/// it later out of context, so it has to stand alone.
const UNATTENDED: &str = "\
You are running unattended, on a schedule, with nobody watching. Three things \
follow. There is no one to answer a question, so make the reasonable \
assumption and say plainly what you assumed rather than stopping to ask. \
Anything that would leave this machine is staged as a draft for the user to \
review later — report it as a draft awaiting their release, never as sent or \
done. And your answer will be read later, out of context, by someone who has \
not seen this conversation: lead with what they need to know, and keep it \
short enough to read on a phone.";

/// Run one trigger to completion and record what happened.
async fn fire(
    global: &GlobalOpts,
    store: &TriggerStore,
    t: &Trigger,
    slot: Option<DateTime<Utc>>,
    manual: bool,
    stop: Option<&CancellationToken>,
) -> Result<()> {
    let mut record = RunRecord::started(&t.name, slot, manual);
    eprintln!("mecha: firing `{}`", t.name);
    // Advisory state for anything that displays "is it running" — never the
    // lock, which is what actually enforces one-at-a-time. Cleared on every
    // path out, including the failure one.
    let _ = store.mark_running(&t.name, slot);

    match run_agent(global, t, &mut record, stop).await {
        Ok(text) => {
            record.status = RunStatus::Ok;
            record.summary = first_line(&text);
            record.notify_error = notify(t, &workspace_of(t), &text);
        }
        Err(e) => {
            record.status = RunStatus::Error;
            record.error = Some(format!("{e:#}"));
            eprintln!("mecha: trigger `{}` failed — {e:#}", t.name);
        }
    }
    record.finished_at = Some(Utc::now());
    store.clear_running(&t.name);
    store.append_run(&record)?;
    Ok(())
}

async fn run_agent(
    global: &GlobalOpts,
    t: &Trigger,
    record: &mut RunRecord,
    stop: Option<&CancellationToken>,
) -> Result<String> {
    check_cost_cap(t)?;

    // The global config only — a scheduled run must not inherit the tool
    // surface of whatever repository the daemon was started in.
    let cfg = mecha_core::config::Config::load_global()?;
    let base = cfg.agent.resolve_system_prompt()?.unwrap_or_default();
    let system = if base.is_empty() {
        UNATTENDED.to_string()
    } else {
        format!("{base}\n\n{UNATTENDED}")
    };

    // `add` writes the workspace down, but triggers authored before it did are
    // on disk with the field unset — and unset is exactly the case that jailed
    // to `$HOME`. Resolve it here too, so an existing trigger is fixed by
    // upgrading rather than by remembering to edit it.
    let workspace = match &t.workspace {
        Some(w) => w.clone(),
        None => mecha_core::work::ensure(&t.name)?,
    };

    let opts = GlobalOpts {
        provider: t.provider.clone().or_else(|| global.provider.clone()),
        model: t.model.clone().or_else(|| global.model.clone()),
        system: Some(system),
        workspace: Some(workspace),
        // The trigger's own policy, never the config's `ask` — see the module
        // docs on trigger.rs. `ask` stays expressible and means "read-only plus
        // a denial that says so", which is what ModeApprover already does.
        yes: t.permission_mode == mecha_core::config::PermissionMode::Allow,
        read_only: t.permission_mode == mecha_core::config::PermissionMode::ReadOnly,
        max_turns: t.max_turns,
        max_output_tokens: t.max_output_tokens,
        max_cost: t.max_cost_usd,
        tools: t.tools.clone(),
        tools_from_trigger: !t.tools.is_empty(),
        // Default closed: a scheduled run carries only the skills its file
        // names. See `Trigger::skills` for why this is the opposite of the
        // `tools` allowlist directly above it.
        skills: t.skills.clone(),
        no_skills: t.skills.is_empty(),
        no_mcp: t.no_mcp,
        global_config_only: true,
        ..GlobalOpts::default()
    };

    // Not interactive: no terminal approver, and no `ask_user` in the registry
    // — that tool is only ever registered by a front-end that owns a human.
    let prepared = setup::prepare(&opts, false).await?;

    let session = Session::create(
        &Session::default_dir()?,
        SessionMeta {
            id: Session::new_id(),
            created_at: Utc::now(),
            provider: prepared.provider_name.clone(),
            model: prepared.model.clone(),
            workspace: prepared.workspace.clone(),
            title: Some(format!("trigger: {}", t.name)),
            kind: Some(mecha_core::session::SessionKind::Trigger),
        },
    )?;
    record.session_id = Some(session.meta.id.clone());
    session.append(&Record::Config(RunConfig::of(
        &prepared.agent,
        &prepared.config,
        &prepared.provider_name,
        &prepared.levers_off,
        Some(&prepared.rules),
    )))?;
    if let Some(route) = &prepared.agent.context().outbox {
        route.set_session_id(&session.meta.id);
    }
    // The trigger's name is its producer, so `message_send` to this name
    // reaches tomorrow's run of it — and this run claims whatever was left
    // for it since last time. Unattended, so the resolved inbound default is
    // accept; the marker is what `mecha msg agents` answers with.
    if let Some(mb) = &prepared.mailbox {
        mb.attach(&t.name, &session.meta.id);
    }

    // A fresh conversation, so nothing — including taint — carries over from
    // yesterday's run of the same trigger.
    let mut convo = Conversation::new();
    let user = Message::user(&t.prompt);
    convo.push(user.clone());
    session.append(&Record::Message(user))?;
    let recorded = convo.messages.clone();

    // The timeout cancels rather than aborts: the run stops at the next safe
    // point and keeps its partial answer, exactly as Ctrl-C does. Killing the
    // future would throw away the work and leave a tool mid-call.
    //
    // A child of the daemon's stop token, so a SIGTERM reaches the run itself
    // rather than only the loop around it. Without that, shutdown would take
    // however long the run had left — up to the trigger's whole timeout — and
    // systemd would SIGKILL it, losing the partial answer and the ledger row
    // that says what happened.
    let token = stop.map(CancellationToken::child_token).unwrap_or_default();
    let cx = RunContext::clone(prepared.agent.context()).with_cancel(token.clone());
    let limit = t
        .timeout_duration()
        .to_std()
        .unwrap_or(std::time::Duration::from_secs(1200));
    let timer = {
        let token = token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(limit) => token.cancel(),
                _ = token.cancelled() => {}
            }
        })
    };

    // The third way a run can stop: someone asked it to, from another process
    // — `mecha trigger cancel`, or the TUI. Polling a file rather than taking
    // a signal, because this run may *be* the daemon's process and a SIGTERM
    // there would stop the whole scheduler. Two seconds is well under human
    // patience and costs one `stat` per tick.
    let canceller = {
        let token = token.clone();
        let store = TriggerStore::open_default()?;
        let name = t.name.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                        if store.cancel_requested(&name) {
                            eprintln!("mecha: trigger `{name}` cancelled by request");
                            token.cancel();
                            return;
                        }
                    }
                }
            }
        })
    };

    let outcome = prepared.agent.run_in(&cx, &mut convo, None).await;
    token.cancel();
    let _ = timer.await;
    let _ = canceller.await;

    session.record_run(&recorded, &convo)?;
    session.append(&Record::Taint(convo.taint))?;
    if let Some(mb) = &prepared.mailbox {
        mb.detach(&session.meta.id);
    }

    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => {
            let cx = prepared.agent.context();
            cx.hooks
                .session_end(&session.meta.id, &session.path, &cx.tools.workspace)
                .await;
            return Err(e);
        }
    };
    session.append(&Record::Summary {
        usage: outcome.usage.clone(),
        turns: outcome.turns,
    })?;
    session.record_outcome(&outcome)?;

    record.turns = outcome.turns;
    record.cost_usd = outcome.cost_usd;
    record.blocked_sends = outcome.blocked_sends;
    record.staged = outcome.tool_calls.iter().filter(|c| c.staged).count() as u32;
    record.tool_calls = outcome.tool_calls.len() as u32;
    // `unknown` counts: naming a tool that does not exist is the environment
    // refusing the call, and it costs the same turn. A denial does not — a
    // policy said no, which is the harness working — and that exclusion must
    // be written out, because a denied trace sets `is_error` as well. Without
    // it the shipped read-only `morning` trigger, which denies every call
    // outside its allowlist, raises a doctor finding for behaving correctly.
    record.tool_errors = outcome
        .tool_calls
        .iter()
        .filter(|c| c.unknown || (c.is_error && !c.denied))
        .count() as u32;
    record.ended_on_failed_call = outcome.ended_on_failed_call;
    record.taint = outcome.taint;
    // Recorded only when it is news: `Completed` is the ordinary case, and a
    // field that is always set is a field nobody reads.
    record.stop_cause = (outcome.stop_cause != mecha_core::agent::StopCause::Completed)
        .then_some(outcome.stop_cause);

    // Same as every other front-end: closing the session is what feeds
    // reflect-on-close, so a trigger's runs are mined like any other.
    let cx = prepared.agent.context();
    cx.hooks
        .session_end(&session.meta.id, &session.path, &cx.tools.workspace)
        .await;

    if outcome.stop_cause.is_early() {
        eprintln!(
            "mecha: trigger `{}` {} — the answer may be incomplete",
            t.name,
            outcome.stop_cause.describe()
        );
    }
    Ok(outcome.text)
}

/// Where a run of this trigger is jailed, and therefore where `notify` runs.
/// Resolved the same way `run_agent` resolves it, so the two cannot disagree
/// about which directory the answer belongs beside.
fn workspace_of(t: &Trigger) -> std::path::PathBuf {
    t.workspace
        .clone()
        .or_else(|| mecha_core::work::ensure(&t.name).ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// Delivery. An observer, like a `post_tool` hook: its failure never fails the
/// run, because the answer is already safe in the transcript.
///
/// It does, however, go **into the ledger** rather than only onto stderr. This
/// is the same argument `stop_cause` exists for: a run whose delivery failed
/// records as plain `ok`, and a trigger that has quietly not rendered its
/// briefing for a week looks exactly like one that works. Nobody reads
/// `journalctl` for a thing that is supposed to be unattended. Returns `None`
/// when there was nothing to do or it worked.
///
/// Run **in the run's workspace**, which is what hooks already do and what
/// `notify` should always have done. It inherited the daemon's directory
/// instead — and the shipped unit sets `WorkingDirectory=%h`, so every notify
/// command had to spell out an absolute path or write into the home directory.
/// That is how the morning briefing ended up doing `mkdir -p ~/.mecha/briefings
/// && cat > …`, dumping its output somewhere outside every path jail where no
/// later run could read it back.
fn notify(t: &Trigger, workspace: &std::path::Path, text: &str) -> Option<String> {
    let command = t.notify.as_ref()?;
    let spawned = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(workspace)
        .stdin(std::process::Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let failure = format!("failed to start: {e}");
            eprintln!("mecha: notify command for `{}` {failure}", t.name);
            return Some(failure);
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    match child.wait() {
        Ok(status) if !status.success() => {
            // 127 is a command that is not on `PATH`, which is the failure this
            // reporting was added for: a systemd unit gives its children a
            // minimal environment, so a `notify` calling anything installed in
            // `~/.cargo/bin` works by hand and not under the daemon.
            let hint = if status.code() == Some(127) {
                " — 127 usually means a command was not found on PATH"
            } else {
                ""
            };
            let failure = format!("exited {status}{hint}");
            eprintln!("mecha: notify command for `{}` {failure}", t.name);
            Some(failure)
        }
        Err(e) => {
            let failure = format!("failed: {e}");
            eprintln!("mecha: notify command for `{}` {failure}", t.name);
            Some(failure)
        }
        _ => None,
    }
}

// ------------------------------------------------------------------ printing

fn local(at: DateTime<Utc>, tz: Tz) -> String {
    at.with_timezone(&tz)
        .format("%a %-d %b %H:%M %Z")
        .to_string()
}

fn human_gap(d: chrono::Duration) -> String {
    let secs = d.num_seconds().abs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3600)
    }
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > 100 {
        format!("{}…", line.chars().take(100).collect::<String>())
    } else {
        line.to_string()
    }
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {

    /// The unit has to name the binary that printed it, by absolute path.
    ///
    /// A unit saying `ExecStart=mecha` resolves against whatever PATH systemd
    /// happens to have, which is rarely the one the person installing it had —
    /// and a scheduler pointed at a different binary than the one printing its
    /// own unit is the version-skew trap that cost 2026-08-21, one layer down.
    #[test]
    fn the_printed_unit_names_this_binary_absolutely() {
        let unit = super::systemd_unit().unwrap();
        let exe = std::env::current_exe().unwrap();
        let exe = exe.display().to_string();
        assert!(
            unit.contains(&format!("ExecStart={exe} trigger daemon")),
            "unit must name this binary by absolute path:\n{unit}"
        );
        for required in ["[Unit]", "[Service]", "[Install]", "WantedBy="] {
            assert!(unit.contains(required), "missing {required}:\n{unit}");
        }
        // Linger, because a user unit dies at logout otherwise and the
        // symptom is a scheduler that works until you close the ssh session.
        assert!(unit.contains("enable-linger"), "{unit}");
    }

    use super::*;
    use clap::CommandFactory;

    fn trigger_with_notify(command: &str) -> Trigger {
        let mut t = Trigger::new("t", "0 7 * * *".parse().unwrap(), "p");
        t.notify = Some(command.to_string());
        t
    }

    /// The failure that actually happened on the first real scheduled run: a
    /// systemd unit hands its children a minimal environment, so a `notify`
    /// calling anything installed by cargo exits 127 under the daemon while
    /// working perfectly by hand.
    ///
    /// The unit now sets `PATH`, which is the fix. This is the *reporting* —
    /// because the run itself succeeds either way, and a briefing that has
    /// quietly not rendered for a week has to look different from one that has.
    #[test]
    fn a_notify_that_could_not_run_is_reported_rather_than_swallowed() {
        let dir = std::env::temp_dir();

        let failure = notify(
            &trigger_with_notify("definitely-not-a-real-command-3f9a"),
            &dir,
            "the answer",
        )
        .expect("a command that is not on PATH has to be reported");
        assert!(failure.contains("127"), "{failure}");
        assert!(
            failure.contains("PATH"),
            "the hint names the actual cause: {failure}"
        );

        // A command that runs and fails is reported too, without the hint —
        // which would be a wrong explanation.
        let failure = notify(&trigger_with_notify("exit 3"), &dir, "x").unwrap();
        assert!(failure.contains("exit"), "{failure}");
        assert!(!failure.contains("PATH"), "{failure}");

        // And silence when it worked, or every successful run would carry a
        // line saying something went wrong.
        assert!(notify(&trigger_with_notify("true"), &dir, "x").is_none());
        assert!(notify(
            &Trigger::new("t", "0 7 * * *".parse().unwrap(), "p"),
            &dir,
            "x"
        )
        .is_none());
    }

    /// `notify` delivers the answer on stdin and runs **in the run's
    /// workspace**, which is what makes a relative path in the command mean the
    /// place the run's own files are.
    #[test]
    fn notify_writes_the_answer_into_the_runs_workspace() {
        let workspace = std::env::temp_dir().join(format!("mecha-notify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workspace);
        std::fs::create_dir_all(&workspace).unwrap();

        assert!(notify(
            &trigger_with_notify("cat > out.md"),
            &workspace,
            "the briefing"
        )
        .is_none());
        assert_eq!(
            std::fs::read_to_string(workspace.join("out.md")).unwrap(),
            "the briefing"
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// A short flag that collides with a global one is a **runtime panic**, on
    /// the subcommand that has it and nowhere else — so `mecha trigger add`
    /// can be uninvokable while every other command works and the whole suite
    /// passes. That is what `-s` for `--schedule` did, colliding with the
    /// global `--system`. `debug_assert` runs clap's own consistency checks
    /// over the entire command tree; it costs one test and covers every
    /// subcommand this project will ever add.
    #[test]
    fn the_cli_has_no_conflicting_flags() {
        crate::Cli::command().debug_assert();
    }

    #[test]
    fn the_unattended_preamble_says_the_three_things_that_change_the_answer() {
        // Each of these corresponds to a way an unattended run goes wrong: it
        // asks a question nobody will answer, it reports a staged draft as
        // sent, or it writes an answer that only makes sense to someone who
        // watched it work.
        assert!(UNATTENDED.contains("no one to answer"));
        assert!(UNATTENDED.contains("never as sent"));
        assert!(UNATTENDED.contains("out of context"));
        // And it must not tell the model to use its best judgment on missing
        // information — measured on this project's own eval set, that wording
        // makes models invent. See `ask_user`'s decline text.
        assert!(!UNATTENDED.contains("best judgment"));
    }

    #[test]
    fn gaps_read_the_way_a_person_would_say_them() {
        assert_eq!(human_gap(chrono::Duration::seconds(30)), "30s");
        assert_eq!(human_gap(chrono::Duration::minutes(5)), "5m");
        assert_eq!(human_gap(chrono::Duration::minutes(90)), "1h 30m");
        assert_eq!(human_gap(chrono::Duration::hours(30)), "1d 6h");
    }

    #[test]
    fn a_summary_is_one_line_and_bounded() {
        assert_eq!(first_line("\n\nthe answer\nmore"), "the answer");
        assert_eq!(first_line(&"x".repeat(200)).chars().count(), 101);
    }
}
