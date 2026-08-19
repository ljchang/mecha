//! `mecha mail` — the triage queue over the mailbox.
//!
//! The command line does everything first and the modal will drive it, on the
//! front door's rule: one implementation, and no way for a UI to do something
//! the terminal cannot.
//!
//! The verbs split along the quarantine, exactly as `mecha frontdoor`'s do.
//! **`list` prints typed fields only** — what the classifier decided, never
//! what anybody wrote — so a subject line cannot reach a terminal that is
//! rendering a table. **`show` prints the prose, deliberately**, because a
//! person reading their own mail in a terminal is the safe context; you cannot
//! be prompt-injected into mailing your own calendar somewhere.
//! **`classify` is the quarantined pass.**
//!
//! Mail arrives the same way it does for the model: through the MCP tool
//! surface. This crate has no dependency on `mecha-mail` and does not gain
//! one here — `mail_recent` already answers in JSON, so the driver reads the
//! same bytes the loop would.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use mecha_core::mail_triage::{
    changed_fields, handle, needs_body, prefilter, Bucket, Correcting, Graded, Proposed, Record,
    Scorecard, ThreadInput, TriageStore, Urgency, Verdict, BODY_CHARS_MAX, CLASSIFIED, DISMISSED,
    FAILED, REQUEST_TYPES,
};

use crate::{setup, GlobalOpts};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// What needs you, newest first (default). Typed fields only.
    List {
        /// Include threads already acted on, and the ones classified `ignore`.
        #[arg(long)]
        all: bool,
        /// Day two: `respond` threads old enough to have been answered and
        /// still untouched.
        ///
        /// **This is the list the morning briefing reads.** A thread
        /// unanswered after a day is overwhelmingly unlikely ever to be
        /// answered (`MAIL-CORPUS-RESEARCH.md` §3), and by then the person has
        /// stopped looking at mail — so the queue being a pull surface is
        /// exactly why the threads die. Keys on the bucket and never on
        /// silence: most unanswered mail correctly needed no reply.
        #[arg(long)]
        aged: bool,
        /// With `--aged`, how old a thread must be. A working day rather than
        /// a literal 24 hours, so an evening email is not nagged about at
        /// breakfast.
        #[arg(long, default_value_t = 30)]
        aged_hours: i64,
        /// With `--aged`, record that these were surfaced so they are not
        /// surfaced again.
        ///
        /// **Separate from reading them on purpose.** A list command that
        /// mutates as a side effect of being run cannot be used to look, and
        /// looking is most of what anyone does with this. The briefing passes
        /// it; a person checking what day two would say does not.
        #[arg(long)]
        surface: bool,
        /// Machine output.
        #[arg(long)]
        json: bool,
    },
    /// Read one thread — the prose, for a human.
    Show {
        thread_id: String,
        #[arg(long)]
        account: Option<String>,
    },
    /// Classify recent mail that has not been classified yet.
    Classify {
        /// One mailbox. Omit to sweep every configured account.
        #[arg(long)]
        account: Option<String>,
        /// How many recent threads to consider per account.
        #[arg(long, default_value_t = 25)]
        limit: u32,
        /// Re-classify threads already in the store.
        #[arg(long)]
        force: bool,
        /// Say what would be classified, and spend nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Drop a thread from the queue without acting on it.
    Dismiss {
        thread_id: String,
        #[arg(long)]
        account: Option<String>,
    },
    /// Say the classifier got something wrong, field by field.
    ///
    /// **Field-level on purpose**: a misread bucket, a missed deadline and a
    /// wrong request kind are different errors with different fixes, and a
    /// correction that only says "this was wrong" teaches the learner noise.
    ///
    /// The verdict is fixed immediately — the list you read is right straight
    /// away — and the before/after pair is kept on the record, because the
    /// mistake is what a learner has to see. Use `none` to clear a deadline or
    /// a request type.
    Correct {
        thread_id: String,
        #[arg(long)]
        account: Option<String>,
        /// respond | notify | ignore
        #[arg(long)]
        bucket: Option<String>,
        /// now | today | week | none
        #[arg(long)]
        urgency: Option<String>,
        /// reply | archive | spam | schedule | task | forward | none
        #[arg(long)]
        proposed: Option<String>,
        /// A kind from the closed list, or `none` to clear it.
        #[arg(long)]
        request_type: Option<String>,
        /// YYYY-MM-DD, or `none` to clear it.
        #[arg(long)]
        deadline: Option<String>,
    },
    /// Archive a thread — out of the inbox, reversible, nobody else notified.
    Archive {
        thread_id: String,
        #[arg(long)]
        account: Option<String>,
    },
    /// Mark a thread as spam.
    ///
    /// **Separate verbs rather than `triage --action <x>`**, on
    /// `SLACK-ACTIONS-DESIGN.md` §1's reasoning: a free-form label argument
    /// would put `spam` inside a verb that reads as harmless. `mecha mail
    /// spam` reads as what it is, and it is the one triage action with an
    /// effect outside the user's own mailbox — it trains the provider's
    /// filter, which archiving does not.
    Spam {
        thread_id: String,
        #[arg(long)]
        account: Option<String>,
    },
    /// Track a thread as a task on the knowledge graph's board.
    ///
    /// **The deadline the classifier found is carried across**, which is the
    /// point: a task someone has to re-read the mail to schedule is a task
    /// they will schedule later or never.
    Task {
        thread_id: String,
        #[arg(long)]
        account: Option<String>,
        /// The task, phrased as an action. Defaults to the classifier's
        /// one-line summary, which describes the mail rather than the action —
        /// so it is worth overriding when the two differ.
        #[arg(long)]
        name: Option<String>,
        /// YYYY-MM-DD, `today`, `tomorrow` or `+Nd`. Defaults to the deadline
        /// on the verdict, if it found one.
        #[arg(long)]
        due: Option<String>,
        /// GTD context tag. `@email` unless told otherwise.
        #[arg(long, default_value = "@email")]
        context: String,
        /// Parent project. **Must already exist on the graph** — it is passed
        /// through untouched and never invented from the thread, because a
        /// project node conjured out of a subject line is a board nobody can
        /// query.
        #[arg(long)]
        project: Option<String>,
    },
    /// Park a thread until somebody answers, naming what is missing.
    ///
    /// Mail's own `needs-info`, and the surviving half of the front-door idea:
    /// the most useful thing to do with "can you write me a letter?" is ask
    /// the questions that make a good one possible.
    ///
    /// **Not `dismiss`.** Dismissing says "I am not doing this"; parking says
    /// "I have asked and cannot proceed yet". The thread stays the user's
    /// problem.
    NeedsInfo {
        thread_id: String,
        #[arg(long)]
        account: Option<String>,
        /// What you are waiting for, in your own words.
        #[arg(long)]
        missing: String,
    },
    /// Turn corrections into `triage`-domain reflections for the learner.
    ///
    /// One model call per unmined correction, tool-less and history-less like
    /// the classifier it is reasoning about. Most corrections produce nothing:
    /// the frame asks for a rule about a *kind* of mail and says outright that
    /// declining is the common case, because a wrong rule rides in every
    /// future classification and a missing one costs a single verdict.
    ///
    /// Idempotent — each correction is keyed into its own ledger, so a nightly
    /// pass never re-argues the same one.
    Reflect {
        #[arg(long)]
        account: Option<String>,
        /// Say what would be reflected on, and spend nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Score the *live* triage store against what actually happened.
    ///
    /// The ledger triage rules are judged against, and the reason ungated
    /// learning is safe in this domain: a rule that starts burying answered
    /// mail regresses a number here rather than waiting for someone to notice.
    ///
    /// **Reply evidence comes from a corpus window, not from `mail_get_thread`.**
    /// That tool renders prose for a model to read, and a measurement keyed on
    /// a display format breaks silently the day the format changes. The corpus
    /// walks all folders including Sent and writes structured rows, so the join
    /// is on `thread_id` against data that was never formatted for anybody.
    /// Refresh it first:
    ///
    /// ```text
    /// mecha-mail corpus --since $(date -d '30 days ago' +%F) --account dartmouth
    /// mecha mail score
    /// ```
    Score {
        #[arg(long, default_value = "dartmouth")]
        account: String,
        /// Only score threads at least this many hours old.
        ///
        /// **A thread younger than this has no outcome yet, and counting it as
        /// unanswered would be manufacturing evidence.** The day-one cliff
        /// (`MAIL-CORPUS-RESEARCH.md` §3) puts 59% of every reply that ever
        /// happens inside the first day, so 48 hours is comfortably past the
        /// point where silence means something. Scoring same-day threads
        /// reports a reply rate of nearly zero and would punish every rule
        /// equally for the passage of time.
        #[arg(long, default_value_t = 48)]
        min_age_hours: i64,
        /// Machine output.
        #[arg(long)]
        json: bool,
    },
    /// Grade the classifier against a corpus of mail whose outcome is known.
    ///
    /// **The ground truth is one-sided and the output says so.** A thread the
    /// user answered proves the thread mattered, so burying it is a countable
    /// error; a thread they never answered proves nothing, because most
    /// unanswered mail correctly needed no answer and some was settled in a
    /// meeting. So this reports a false-`ignore` rate on the answered stratum
    /// and a *volume* on the other, and never a single blended accuracy.
    ///
    /// Reads `~/.mecha/mail-corpus/<account>.jsonl` — see `mecha-mail corpus`.
    /// Writes nothing to the triage store: grading year-old mail is not
    /// triaging it, and a scorecard that mutates the queue it measures would
    /// be unrepeatable.
    Eval {
        /// Which corpus file to grade.
        #[arg(long, default_value = "dartmouth")]
        account: String,
        /// How many threads to sample from each stratum. Answered threads are
        /// rare, so both strata are sampled to this size rather than the
        /// corpus being sampled uniformly — otherwise a run of 200 would hold
        /// a handful of the only threads that carry ground truth.
        #[arg(long, default_value_t = 60)]
        sample: usize,
        /// Fixed by default so a scorecard is reproducible.
        #[arg(long, default_value_t = 7)]
        seed: u64,
        /// Report what the deterministic rules do and stop. No model, no cost.
        #[arg(long)]
        prefilter_only: bool,
        /// Machine output.
        #[arg(long)]
        json: bool,
        /// Write every graded thread here as JSONL — the verdict, the bucket,
        /// and whether it was answered.
        ///
        /// **A measurement that discards its evidence has to be re-run to be
        /// re-read.** The first run of this eval reported a merged "surfaced"
        /// figure and threw away the 120 judgements behind it, so splitting
        /// `respond` from `notify` afterwards cost another hour of inference
        /// rather than a `grep`. Grading the artifact is this project's rule
        /// for models; it applies to its own instruments too.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd.unwrap_or(Cmd::List {
        all: false,
        aged: false,
        aged_hours: 30,
        surface: false,
        json: false,
    }) {
        Cmd::List {
            all,
            aged,
            aged_hours,
            surface,
            json,
        } => list(all, aged, aged_hours, surface, json),
        Cmd::Show { thread_id, account } => show(global, &thread_id, account.as_deref()).await,
        Cmd::Classify {
            account,
            limit,
            force,
            dry_run,
        } => classify(global, account.as_deref(), limit, force, dry_run).await,
        Cmd::Dismiss { thread_id, account } => dismiss(&thread_id, account.as_deref()),
        Cmd::Correct {
            thread_id,
            account,
            bucket,
            urgency,
            proposed,
            request_type,
            deadline,
        } => correct(
            &thread_id,
            account.as_deref(),
            bucket.as_deref(),
            urgency.as_deref(),
            proposed.as_deref(),
            request_type.as_deref(),
            deadline.as_deref(),
        ),
        Cmd::Archive { thread_id, account } => {
            triage(global, &thread_id, account.as_deref(), "archive").await
        }
        Cmd::Spam { thread_id, account } => {
            triage(global, &thread_id, account.as_deref(), "spam").await
        }
        Cmd::Task {
            thread_id,
            account,
            name,
            due,
            context,
            project,
        } => {
            task(
                global,
                &thread_id,
                account.as_deref(),
                name.as_deref(),
                due.as_deref(),
                &context,
                project.as_deref(),
            )
            .await
        }
        Cmd::NeedsInfo {
            thread_id,
            account,
            missing,
        } => needs_info(&thread_id, account.as_deref(), &missing),
        Cmd::Reflect { account, dry_run } => reflect(global, account.as_deref(), dry_run).await,
        Cmd::Score {
            account,
            min_age_hours,
            json,
        } => score(&account, min_age_hours, json),
        Cmd::Eval {
            account,
            sample,
            seed,
            prefilter_only,
            json,
            out,
        } => eval(global, &account, sample, seed, prefilter_only, json, out).await,
    }
}

/// Find a tool by its bare name whatever prefix config gave the server.
///
/// `mail__mail_recent` assumes the server is aliased `mail` with
/// `prefix_tools` on, and neither is guaranteed — a deployment that renamed
/// the server would get "tool not available" from a driver that hardcoded the
/// prefix. Matching on the suffix is what `[outbox] tools` already does when
/// it warns about a routed name.
fn find_tool<'a>(
    registry: &'a mecha_core::tool::Registry,
    bare: &str,
) -> Option<&'a std::sync::Arc<dyn mecha_core::tool::Tool>> {
    registry
        .iter()
        .find(|t| t.name() == bare || t.name().ends_with(&format!("__{bare}")))
}

fn list(all: bool, aged: bool, aged_hours: i64, surface: bool, as_json: bool) -> Result<()> {
    let Some(store) = TriageStore::open_existing_default() else {
        println!("nothing classified yet — run `mecha mail classify`");
        return Ok(());
    };
    let now = chrono::Utc::now().to_rfc3339();
    let rows: Vec<Record> = store
        .list()?
        .into_iter()
        .filter(|r| {
            if aged {
                r.day_two_candidate(&now, aged_hours)
            } else {
                all || r.needs_me() || r.state == FAILED
            }
        })
        .collect();

    // Marking is the caller's explicit request, and it happens before the
    // display so a broken pipe cannot surface a thread twice.
    if aged && surface {
        for r in &rows {
            let mut r = r.clone();
            r.rest
                .insert(mecha_core::mail_triage::SURFACED_AT.to_string(), json!(now));
            store.put(&r)?;
        }
    }

    if as_json {
        // The typed view, not the record: `list --json` is what a script or a
        // modal reads, and neither has a human's excuse for seeing the prose.
        let out: Vec<Value> = rows.iter().map(|r| r.for_privileged_run()).collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if rows.is_empty() {
        if !aged {
            println!("nothing needs you");
            return Ok(());
        }
        // **"Nothing new to surface" is not "nothing is waiting."** Day two
        // says its piece once, so after a briefing the same threads are still
        // unanswered and no longer candidates. Reporting that as "every thread
        // got dealt with" would be a cheerful lie about the exact backlog this
        // feature exists to expose.
        let still: usize = store
            .list()?
            .into_iter()
            .filter(|r| {
                r.state == CLASSIFIED
                    && r.rest.contains_key(mecha_core::mail_triage::SURFACED_AT)
                    && r.verdict
                        .as_ref()
                        .is_some_and(|v| v.bucket == mecha_core::mail_triage::Bucket::Respond)
            })
            .count();
        match still {
            0 => println!("nothing has been waiting"),
            n => println!(
                "nothing new to surface — {n} thread(s) are still unanswered but \
                 have had their turn. `mecha mail list` shows them."
            ),
        }
        return Ok(());
    }
    if aged {
        println!(
            "{} thread(s) you meant to answer and have not, {aged_hours}h+ old:\n",
            rows.len()
        );
        // Compact by construction: this list is read in a briefing, and a
        // seventy-six-character id twice a thread is most of the section. The
        // handle is enough for every verb — they resolve a unique suffix — and
        // `--json` still carries the whole id for anything mechanical.
        for r in &rows {
            let v = r.verdict.as_ref();
            println!(
                "  {:<9} {:<8} {}",
                v.map(|v| v.urgency.as_str()).unwrap_or(""),
                mecha_core::mail_triage::handle(&r.thread_id),
                v.map(|v| v.one_line.as_str()).unwrap_or(&r.subject),
            );
            println!("             {:<8} {}", "", r.from);
        }
        println!(
            "\n`mecha mail show <handle>` reads one · `reply`, `task`, `needs-info` \
             and `correct` all take a handle too."
        );
        return Ok(());
    }
    for r in &rows {
        match (&r.verdict, r.state.as_str()) {
            (_, FAILED) => println!(
                "  !  {:<10} {:<9} classification failed — {}",
                r.account,
                "",
                r.error.as_deref().unwrap_or("no reason recorded")
            ),
            (Some(v), _) => {
                let mark = if v.bucket == mecha_core::mail_triage::Bucket::Respond {
                    "●"
                } else {
                    " "
                };
                let tags = if v.tags.is_empty() {
                    String::new()
                } else {
                    format!("#{}", v.tags.join(" #"))
                };
                println!(
                    "  {mark} {:<7} {:<10} {:<12} {}",
                    v.urgency.as_str(),
                    r.account,
                    tags,
                    v.one_line
                );
                println!(
                    "      {} · {} · proposed: {}{}",
                    r.thread_id,
                    r.from,
                    v.proposed.as_str(),
                    v.deadline
                        .as_deref()
                        .map(|d| format!(" · due {d}"))
                        .unwrap_or_default()
                );
            }
            (None, _) => println!("  ?  {:<10} {} (no verdict)", r.account, r.thread_id),
        }
    }
    println!(
        "\n{} thread(s). `mecha mail show <thread_id>` to read one.",
        rows.len()
    );
    Ok(())
}

/// Print the prose. The one verb that does, and it is for a person.
async fn show(global: &GlobalOpts, thread_id: &str, account: Option<&str>) -> Result<()> {
    // A handle from a briefing has to work here too, and the failure without
    // this is not local: the handle goes to the provider, which answers
    // `ErrorInvalidIdMalformed` — an API error for what is really a typo.
    let thread_id = &match TriageStore::open_existing_default() {
        Some(store) => resolve_thread_lenient(&store, thread_id)?,
        None => thread_id.to_string(),
    };
    let store = TriageStore::open_existing_default();
    let rec = store
        .as_ref()
        .and_then(|s| account.and_then(|a| s.get(a, thread_id)));

    if let Some(r) = &rec {
        println!("account:   {}", r.account);
        println!("from:      {} <{}>", r.from_name, r.from);
        println!("subject:   {}", r.subject);
        println!("date:      {}", r.date);
        if let Some(v) = &r.verdict {
            println!(
                "verdict:   {} · {} · proposed {}",
                v.bucket.as_str(),
                v.urgency.as_str(),
                v.proposed.as_str()
            );
            if !v.tags.is_empty() {
                println!("tags:      #{}", v.tags.join(" #"));
            }
            if let Some(rt) = &v.request_type {
                println!("looks like a `{rt}` request arriving as email");
            }
            // The classifier's own words are shown here and nowhere a run can
            // reach — the whole reason `for_privileged_run` withholds them.
            println!("reasoning: {}", v.reasoning);
        }
        println!();
    }

    let prepared = setup::prepare_tools(global, false).await?;
    let Some(tool) = find_tool(&prepared.registry, "mail_get_thread") else {
        bail!("no mail server in this configuration — is `[[mcp]]` for mecha-mail enabled?");
    };
    let mut input = json!({ "thread_id": thread_id });
    if let Some(a) = account.or(rec.as_ref().map(|r| r.account.as_str())) {
        input["account"] = json!(a);
    }
    let ctx = tool_ctx(&prepared);
    let out = tool.call(input, &ctx).await?;
    println!("{}", out.content);
    Ok(())
}

fn tool_ctx(prepared: &setup::PreparedTools) -> mecha_core::tool::ToolCtx {
    mecha_core::tool::ToolCtx {
        workspace: prepared.workspace.clone(),
        shell_timeout: std::time::Duration::from_secs(prepared.config.tools.shell_timeout_secs),
        security: prepared.config.security.clone(),
        output_budget_bytes: prepared.config.tools.resolved_output_budget(None),
        ..Default::default()
    }
}

fn dismiss(thread_id: &str, account: Option<&str>) -> Result<()> {
    let Some(store) = TriageStore::open_existing_default() else {
        bail!("nothing classified yet");
    };
    let thread_id = &resolve_thread(&store, thread_id)?;
    let account = match account {
        Some(a) => a.to_string(),
        None => {
            // One unambiguous match is a convenience; several is a question.
            let hits: Vec<Record> = store
                .list()?
                .into_iter()
                .filter(|r| r.thread_id == *thread_id)
                .collect();
            match hits.len() {
                1 => hits[0].account.clone(),
                0 => bail!("no classified thread `{thread_id}`"),
                _ => bail!("thread `{thread_id}` exists in several accounts — pass --account"),
            }
        }
    };
    if store.mark(&account, thread_id, "dismiss", DISMISSED)? {
        println!("dismissed {thread_id} ({account})");
    } else {
        bail!("no classified thread `{thread_id}` in `{account}`");
    }
    Ok(())
}

/// The sweep: read recent mail, classify what is new, write verdicts.
///
/// Every thread is its own isolated call. Nothing accumulates across them —
/// no conversation, no shared prefix — so one hostile message cannot colour
/// the reading of the next, and a failure is one row rather than the batch.
async fn classify(
    global: &GlobalOpts,
    account: Option<&str>,
    limit: u32,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let store = TriageStore::open(TriageStore::default_root()?)?;

    // The mail surface first, because failing here should cost no model call.
    let prepared = setup::prepare_tools(global, false).await?;
    let Some(recent) = find_tool(&prepared.registry, "mail_recent") else {
        bail!("no mail server in this configuration — is `[[mcp]]` for mecha-mail enabled?");
    };
    let ctx = tool_ctx(&prepared);
    let mut input = json!({ "max_results": limit.clamp(1, 50) });
    if let Some(a) = account {
        input["account"] = json!(a);
    }
    let out = recent.call(input, &ctx).await?;
    if out.is_error {
        bail!("reading mail failed: {}", out.content);
    }
    let rows: Vec<Value> =
        serde_json::from_str(&out.content).context("mail_recent did not answer with JSON rows")?;

    let todo: Vec<&Value> = rows
        .iter()
        .filter(|r| {
            let (Some(a), Some(t)) = (r["account"].as_str(), r["thread_id"].as_str()) else {
                return false;
            };
            // `needs_classifying`, not `!is_known`: a record left in `failed`
            // by an outage still needs classifying, and skipping it would
            // bury the thread permanently.
            force || store.needs_classifying(a, t)
        })
        .collect();

    println!(
        "{} thread(s) read, {} to classify{}",
        rows.len(),
        todo.len(),
        if dry_run { " (dry run)" } else { "" }
    );
    if todo.is_empty() || dry_run {
        for r in &todo {
            println!("  would classify {} — {}", r["thread_id"], r["subject"]);
        }
        return Ok(());
    }

    // A provider and nothing else. Building an agent here would mean the
    // quarantine had a tool surface to be talked into using.
    let cwd = std::env::current_dir()?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| provider.default_model().to_string());
    // The classifier has no clock, and a deadline judged in the wrong zone is
    // wrong in the way that reads as correct.
    let today = match cfg.agent.timezone() {
        Some(tz) => chrono::Utc::now()
            .with_timezone(&tz)
            .format("%Y-%m-%d")
            .to_string(),
        None => chrono::Local::now().format("%Y-%m-%d").to_string(),
    };
    // What this recipient has corrected before, newest first. Bounded, because
    // this rides on every classification of every thread — the cheap half of
    // the correction loop only stays cheap if it stays small.
    let examples = mecha_core::mail_triage::select_examples(&store.list()?);
    if !examples.is_empty() {
        eprintln!(
            "{} correction(s) in the classifier's prompt",
            examples.len()
        );
    }
    eprintln!("classifying with {model} ({provider_name})");

    let get_thread = find_tool(&prepared.registry, "mail_get_thread");
    let (mut ok, mut failed, mut escalated) = (0u32, 0u32, 0u32);
    let mut prefiltered = 0u32;
    for row in todo {
        let mut thread = row_to_input(row);

        // Ahead of the model, never instead of it for anything in doubt.
        // About half a real mailbox is bulk or an automated sender, and
        // spending a classifier call on a shipping notification is the cost
        // this removes. The rule only ever produces `ignore` and reads only
        // the envelope — see `mail_triage::prefilter`.
        if let Some((v, rule)) =
            mecha_core::mail_triage::prefilter(&thread, row["bulk"].as_bool().unwrap_or(false))
        {
            if let Err(e) = store.put(&record(&thread, Some(v), None)) {
                eprintln!("  ! {} — {e}", thread.thread_id);
                failed += 1;
            } else {
                prefiltered += 1;
                if global.verbose {
                    println!("  · {} — {} (no model)", thread.subject, rule.as_str());
                }
            }
            continue;
        }

        let verdict = mecha_core::mail_triage::classify_with(
            provider.as_ref(),
            &model,
            &thread,
            &today,
            &examples,
        )
        .await;

        // The second pass. Only where the answer changes what happens — see
        // `needs_body` — and only when the whole thread can actually be
        // fetched: a failed read leaves the snippet verdict standing rather
        // than losing it, because a worse answer beats no answer here.
        let mut from_bucket = None;
        let mut did_escalate = false;
        let mut changed: Vec<String> = Vec::new();
        let verdict = match (&verdict, &get_thread) {
            (Ok(v), Some(tool)) if needs_body(v) => {
                match fetch_body(tool.as_ref(), &ctx, &thread).await {
                    Ok(body) => {
                        thread.body = body;
                        match mecha_core::mail_triage::classify(
                            provider.as_ref(),
                            &model,
                            &thread,
                            &today,
                        )
                        .await
                        {
                            Ok(second) => {
                                escalated += 1;
                                did_escalate = true;
                                changed = changed_fields(v, &second);
                                if second.bucket != v.bucket {
                                    from_bucket = Some(v.bucket.as_str().to_string());
                                }
                                Ok(second)
                            }
                            // The body pass failing is not the snippet pass
                            // being wrong.
                            Err(e) => {
                                eprintln!("      (second pass failed, keeping the first: {e:#})");
                                verdict
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("      (could not read the thread: {e:#})");
                        verdict
                    }
                }
            }
            _ => verdict,
        };

        let rec = match verdict {
            Ok(v) => {
                ok += 1;
                print_line(&thread, &v, from_bucket.as_deref());
                let mut r = record(&thread, Some(v), None);
                r.escalated = did_escalate;
                r.escalated_changed = changed;
                r.escalated_from = from_bucket;
                r
            }
            // A failure is a row and a human's problem. It never falls back to
            // handing the prose on, and it never stops the sweep: one
            // unreadable message must not cost the other twenty-four.
            Err(e) => {
                failed += 1;
                eprintln!("  ! {} — {e:#}", thread.thread_id);
                record(&thread, None, Some(format!("{e:#}")))
            }
        };
        store.put(&rec)?;
    }
    // The pre-filtered count is reported rather than folded into `ok`,
    // because "how much is the cheap rule taking" is the question that decides
    // whether it is too aggressive — and a number nobody can see is a rule
    // nobody can grade.
    println!(
        "\n{ok} classified ({escalated} read in full), \
         {prefiltered} disposed without a model, {failed} failed"
    );

    // **A run that accomplished nothing must exit non-zero.**
    //
    // 2026-08-19: the nightly classified 0 of 16 threads — the local model
    // server was not running, so every call failed — and systemd recorded the
    // unit as SUCCESS, because this function returned `Ok(())` regardless.
    // That is the silently-degrading pattern: `OnFailure=`, `systemctl
    // --failed` and doctor's failed-unit check all read a broken nightly as a
    // healthy one, and the only trace was a log nobody reads.
    //
    // Partial failure stays a success on purpose — fourteen of sixteen
    // classified is a working nightly, and failing the unit for it would
    // train someone to ignore the alarm. What is reported here is the run
    // having done *nothing*: no classification and no pre-filter disposal,
    // with at least one failure to explain why.
    if run_accomplished_nothing(ok, prefiltered, failed) {
        bail!(
            "classified nothing: all {failed} thread(s) failed. \
             The most common cause is the model provider being unreachable — \
             check it is running, then re-run."
        );
    }
    Ok(())
}

/// The whole conversation, as text, for the second pass.
///
/// Capped at [`BODY_CHARS_MAX`] from the *end*, because `mail_get_thread`
/// renders oldest-first and the newest message is the one asking for
/// something — truncating the front of a long thread keeps the part a reply
/// would answer.
async fn fetch_body(
    tool: &dyn mecha_core::tool::Tool,
    ctx: &mecha_core::tool::ToolCtx,
    t: &ThreadInput,
) -> Result<String> {
    let out = tool
        .call(
            json!({ "thread_id": t.thread_id, "account": t.account }),
            ctx,
        )
        .await?;
    if out.is_error {
        bail!("{}", out.content);
    }
    let text = out.content;
    if text.chars().count() <= BODY_CHARS_MAX {
        return Ok(text);
    }
    let skip = text.chars().count() - BODY_CHARS_MAX;
    Ok(format!(
        "[earlier messages omitted]\n{}",
        text.chars().skip(skip).collect::<String>()
    ))
}

fn row_to_input(row: &Value) -> ThreadInput {
    let s = |k: &str| row[k].as_str().unwrap_or_default().to_string();
    // `from` arrives as `Name <addr>`; the address is the half `kg_entity`
    // resolves and the display name is the half a stranger chose.
    let from_full = s("from");
    let (from_name, from) = match (from_full.find('<'), from_full.rfind('>')) {
        (Some(a), Some(b)) if b > a => (
            from_full[..a].trim().to_string(),
            from_full[a + 1..b].trim().to_string(),
        ),
        _ => (String::new(), from_full.clone()),
    };
    ThreadInput {
        thread_id: s("thread_id"),
        account: s("account"),
        from,
        from_name,
        subject: s("subject"),
        date: s("date"),
        // Snippet-first: full bodies classify better and cost far more on a
        // local model. The escalation rule is a later decision, and it is
        // measurable once this store has rows in it.
        body: s("snippet"),
    }
}

fn record(t: &ThreadInput, verdict: Option<Verdict>, error: Option<String>) -> Record {
    Record {
        thread_id: t.thread_id.clone(),
        account: t.account.clone(),
        subject: t.subject.clone(),
        from: t.from.clone(),
        from_name: t.from_name.clone(),
        date: t.date.clone(),
        state: if error.is_some() {
            FAILED.to_string()
        } else {
            CLASSIFIED.to_string()
        },
        verdict,
        error,
        classified_at: chrono::Utc::now().to_rfc3339(),
        escalated: false,
        escalated_changed: Vec::new(),
        escalated_from: None,
        corrections: Vec::new(),
        acted: None,
        acted_at: None,
        rest: Default::default(),
    }
}

fn print_line(t: &ThreadInput, v: &Verdict, escalated_from: Option<&str>) {
    println!(
        "  {:<7} {:<8} {} — {}{}",
        v.urgency.as_str(),
        v.bucket.as_str(),
        t.from,
        v.one_line,
        escalated_from
            .map(|b| format!("  [was {b} on the snippet]"))
            .unwrap_or_default()
    );
}

/// Whether a classify run did nothing at all and should fail its unit.
///
/// A function so the rule is testable without a mailbox: the condition is easy
/// to state and easy to get subtly wrong in a direction nobody notices, which
/// is how the original `Ok(())` survived.
fn run_accomplished_nothing(ok: u32, prefiltered: u32, failed: u32) -> bool {
    ok == 0 && prefiltered == 0 && failed > 0
}

/// One thread reconstructed from the corpus, with the outcome that grades it.
struct CorpusThread {
    input: ThreadInput,
    bulk: bool,
    replied: bool,
}

/// Group corpus messages into threads and recover the ground truth.
///
/// The user's own address comes from the corpus rather than from config: the
/// rows record who sent each message, and a thread counts as answered only
/// when an outbound message *follows* the inbound one. Counting any outbound
/// message would include threads the user started and somebody replied to,
/// which is not an answer to anything and inflates the baseline.
fn corpus_threads(path: &std::path::Path, me: &str) -> Result<Vec<CorpusThread>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {} — run `mecha-mail corpus` first", path.display()))?;
    let mut by: std::collections::HashMap<String, Vec<Value>> = std::collections::HashMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).context("a corpus line is not JSON")?;
        let Some(t) = v["thread_id"].as_str() else {
            continue;
        };
        by.entry(t.to_string()).or_default().push(v);
    }
    let mut out = Vec::new();
    for (thread_id, mut msgs) in by {
        msgs.sort_by(|a, b| a["date"].as_str().cmp(&b["date"].as_str()));
        let is_me = |m: &Value| {
            m["from"]
                .as_str()
                .unwrap_or_default()
                .eq_ignore_ascii_case(me)
        };
        let Some(first_in) = msgs.iter().find(|m| !is_me(m)) else {
            continue; // the user's own thread with no inbound message
        };
        // **A thread the user started is not evidence about replying to mail.**
        // If they sent before the first inbound message, that message is a
        // reply *to them*, and anything they send afterwards is the
        // conversation continuing rather than an answer to an incoming
        // request. Counting those inflated the answered stratum by ~1% of the
        // graded pool and put at least one bogus "false ignore" in the first
        // scorecard — an auto-reply from an office they had emailed first.
        if msgs.iter().any(|m| {
            is_me(m)
                && m["date"].as_str().unwrap_or_default()
                    < first_in["date"].as_str().unwrap_or_default()
        }) {
            continue;
        }
        let after = first_in["date"].as_str().unwrap_or_default().to_string();
        let replied = msgs
            .iter()
            .any(|m| is_me(m) && m["date"].as_str().unwrap_or_default() >= after.as_str());
        let g = |k: &str| first_in[k].as_str().unwrap_or_default().to_string();
        out.push(CorpusThread {
            input: ThreadInput {
                thread_id,
                account: g("account"),
                from: g("from"),
                from_name: g("from_name"),
                subject: g("subject"),
                date: g("date"),
                // The snippet is exactly what the live classifier sees on its
                // first pass, which is the pass this grades.
                body: g("snippet"),
            },
            bulk: first_in["bulk"].as_bool().unwrap_or(false),
            replied,
        });
    }
    Ok(out)
}

/// Deterministic shuffle so a scorecard is reproducible from its seed.
fn shuffled<T>(mut v: Vec<T>, seed: u64) -> Vec<T> {
    let mut st = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (st >> 33) as usize
    };
    for i in (1..v.len()).rev() {
        v.swap(i, next() % (i + 1));
    }
    v
}

async fn eval(
    global: &GlobalOpts,
    account: &str,
    sample: usize,
    seed: u64,
    prefilter_only: bool,
    json_out: bool,
    out_path: Option<std::path::PathBuf>,
) -> Result<()> {
    // Via `mecha_home` rather than `$HOME` directly, so `MECHA_HOME` moves
    // the corpus with every other store.
    let dir = mecha_core::work::mecha_home()?.join("mail-corpus");
    let path = dir.join(format!("{account}.jsonl"));
    let me = std::env::var("MECHA_EVAL_SELF").ok();
    let me = match me {
        Some(m) => m,
        None => guess_self(&path)?,
    };
    let threads = corpus_threads(&path, &me)?;
    if threads.is_empty() {
        bail!("no threads in {}", path.display());
    }

    // The pre-filter is deterministic, so it is graded over the WHOLE corpus
    // rather than a sample. There is no reason to estimate a number that can
    // be computed exactly and for nothing.
    let mut pf_caught = 0usize;
    let mut pf_caught_replied = 0usize;
    let mut survivors: Vec<&CorpusThread> = Vec::new();
    for t in &threads {
        match prefilter(&t.input, t.bulk) {
            Some(_) => {
                pf_caught += 1;
                pf_caught_replied += usize::from(t.replied);
            }
            None => survivors.push(t),
        }
    }
    let total = threads.len();
    println!(
        "corpus {}: {total} threads, self = {me}\n\
         pre-filter: {pf_caught} disposed ({:.1}%), {pf_caught_replied} of them had been answered ({:.2}% of disposed)\n\
         reaching the classifier: {} ({:.1}%)",
        path.display(),
        100.0 * pf_caught as f64 / total as f64,
        100.0 * pf_caught_replied as f64 / pf_caught.max(1) as f64,
        survivors.len(),
        100.0 * survivors.len() as f64 / total as f64,
    );
    if prefilter_only {
        return Ok(());
    }

    // Both strata are sampled to the same size. Answered threads are a small
    // minority, so a uniform sample would spend almost all of its model calls
    // on the stratum that carries no ground truth.
    let (yes, no): (Vec<_>, Vec<_>) = survivors.into_iter().partition(|t| t.replied);
    fn pick(v: Vec<&CorpusThread>, s: u64, n: usize) -> Vec<&CorpusThread> {
        shuffled(v, s).into_iter().take(n).collect()
    }
    let chosen: Vec<&CorpusThread> = pick(yes, seed, sample)
        .into_iter()
        .chain(pick(no, seed ^ 0x9E37_79B9, sample))
        .collect();

    let cwd = std::env::current_dir()?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| provider.default_model().to_string());
    eprintln!(
        "grading {} thread(s) with {model} ({provider_name}) — no writes to the triage store",
        chosen.len()
    );

    let mut graded = Vec::new();
    let mut rows: Vec<Value> = Vec::new();
    let mut failed = 0u32;
    for (i, t) in chosen.iter().enumerate() {
        // Judged as of the day it arrived. Grading a year-old deadline against
        // today would score every one of them as passed.
        let today = t.input.date.get(..10).unwrap_or("1970-01-01");
        match mecha_core::mail_triage::classify(provider.as_ref(), &model, &t.input, today).await {
            Ok(v) => {
                rows.push(json!({
                    "thread_id": t.input.thread_id,
                    "date": t.input.date,
                    "replied": t.replied,
                    "bucket": v.bucket.as_str(),
                    "urgency": v.urgency.as_str(),
                    "proposed": v.proposed.as_str(),
                    "request_type": v.request_type,
                    "deadline": v.deadline,
                    "escalates": mecha_core::mail_triage::needs_body(&v),
                }));
                graded.push(Graded {
                    replied: t.replied,
                    verdict: Some(v),
                    prefiltered: None,
                });
            }
            Err(e) => {
                failed += 1;
                eprintln!("  ! {} — {e}", t.input.thread_id);
            }
        }
        if (i + 1) % 10 == 0 {
            eprint!("\r  {}/{}", i + 1, chosen.len());
        }
    }
    eprintln!();

    // Written before the scorecard is printed: if anything below panics or the
    // terminal scrolls away, an hour of inference is still on disk.
    //
    // Thread ids and dates only — no subject, sender or snippet. The corpus is
    // real correspondence and this file is a measurement artefact, not a copy
    // of the mailbox; anything needing the prose can join back on the id.
    if let Some(p) = &out_path {
        let body: String = rows
            .iter()
            .map(|r| format!("{r}\n"))
            .collect::<Vec<_>>()
            .concat();
        std::fs::write(p, body).with_context(|| format!("writing {}", p.display()))?;
        eprintln!("graded verdicts → {}", p.display());
    }

    let s = Scorecard::of(&graded);
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "corpus": path.display().to_string(),
                "threads_total": total,
                "prefilter": {"disposed": pf_caught, "disposed_but_answered": pf_caught_replied},
                "sampled": graded.len(), "failed": failed,
                "answered": {"n": s.replied, "buried": s.replied_final_ignore,
                             "false_ignore_rate": s.false_ignore_rate(),
                             "respond": s.replied_buckets[0],
                             "notify": s.replied_buckets[1],
                             "ignore": s.replied_buckets[2]},
                "unanswered": {"n": s.unreplied, "surfaced": s.unreplied_surfaced,
                               "respond": s.unreplied_buckets[0],
                               "notify": s.unreplied_buckets[1],
                               "ignore": s.unreplied_buckets[2]},
                "caveat": Scorecard::caveat(),
            }))?
        );
        return Ok(());
    }
    println!("\n── answered threads — the stratum with ground truth ──");
    println!("  graded:            {}", s.replied);
    match s.false_ignore_rate() {
        Some(r) => println!(
            "  buried as `ignore`: {} ({:.1}%)  ← the number this eval exists for",
            s.replied_final_ignore,
            100.0 * r
        ),
        None => println!("  buried as `ignore`: n/a — no answered threads in the sample"),
    }
    let pct = |n: usize, d: usize| 100.0 * n as f64 / d.max(1) as f64;
    println!(
        "  buckets:            respond {} ({:.0}%) · notify {} · ignore {}",
        s.replied_buckets[0],
        pct(s.replied_buckets[0], s.replied),
        s.replied_buckets[1],
        s.replied_buckets[2]
    );
    println!("\n── unanswered threads — no ground truth ──");
    println!("  graded:            {}", s.unreplied);
    println!(
        "  buckets:            respond {} ({:.0}%) · notify {} · ignore {}",
        s.unreplied_buckets[0],
        pct(s.unreplied_buckets[0], s.unreplied),
        s.unreplied_buckets[1],
        s.unreplied_buckets[2]
    );
    println!(
        "  surfaced at all (respond+notify): {} ({:.1}%)",
        s.unreplied_surfaced,
        pct(s.unreplied_surfaced, s.unreplied)
    );
    println!("  {}", Scorecard::caveat());
    println!(
        "  `respond` is what day-two resurfacing would key on — {} of {} here.",
        s.unreplied_buckets[0], s.unreplied
    );
    if failed > 0 {
        println!("\n{failed} thread(s) failed to classify and are excluded.");
    }
    Ok(())
}

/// The mailbox owner's address, read off the corpus: the address that appears
/// most often as a *recipient*. Derived rather than configured because the
/// corpus is the thing being graded, and a mismatch between the two would
/// silently score every thread as unanswered.
fn guess_self(path: &std::path::Path) -> Result<String> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {} — run `mecha-mail corpus` first", path.display()))?;
    let mut c: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for line in text.lines().take(4000) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for r in v["to"].as_array().into_iter().flatten() {
            if let Some(a) = r.as_str() {
                *c.entry(a.to_ascii_lowercase()).or_default() += 1;
            }
        }
    }
    c.into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(a, _)| a)
        .context("could not infer the mailbox owner; set MECHA_EVAL_SELF")
}

#[allow(clippy::too_many_arguments)]
fn correct(
    thread_id: &str,
    account: Option<&str>,
    bucket: Option<&str>,
    urgency: Option<&str>,
    proposed: Option<&str>,
    request_type: Option<&str>,
    deadline: Option<&str>,
) -> Result<()> {
    let mut c = Correcting::default();
    if let Some(v) = bucket {
        c.bucket = Some(one_of(
            "bucket",
            v,
            &[
                ("respond", Bucket::Respond),
                ("notify", Bucket::Notify),
                ("ignore", Bucket::Ignore),
            ],
        )?);
    }
    if let Some(v) = urgency {
        c.urgency = Some(one_of(
            "urgency",
            v,
            &[
                ("now", Urgency::Now),
                ("today", Urgency::Today),
                ("week", Urgency::Week),
                ("none", Urgency::None),
            ],
        )?);
    }
    if let Some(v) = proposed {
        c.proposed = Some(one_of(
            "proposed",
            v,
            &[
                ("reply", Proposed::Reply),
                ("archive", Proposed::Archive),
                ("spam", Proposed::Spam),
                ("schedule", Proposed::Schedule),
                ("task", Proposed::Task),
                ("forward", Proposed::Forward),
                ("none", Proposed::None),
            ],
        )?);
    }
    if let Some(v) = request_type {
        c.request_type = Some(match v {
            "none" => None,
            other => {
                if !REQUEST_TYPES.contains(&other) {
                    bail!(
                        "unknown request type `{other}` — one of: {}, or `none`",
                        REQUEST_TYPES.join(", ")
                    );
                }
                Some(other.to_string())
            }
        });
    }
    if let Some(v) = deadline {
        c.deadline = Some(match v {
            "none" => None,
            d => {
                // The same shape `parse_verdict` enforces: anything downstream
                // hands this to `kg_task_create`, which takes YYYY-MM-DD.
                if chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err() {
                    bail!("deadline `{d}` is not YYYY-MM-DD (or `none`)");
                }
                Some(d.to_string())
            }
        });
    }
    if c.is_empty() {
        bail!("nothing to correct — pass at least one of --bucket, --urgency, --proposed, --request-type, --deadline");
    }

    let store = TriageStore::open(TriageStore::default_root()?)?;
    let thread_id = &resolve_thread(&store, thread_id)?;
    let account = resolve_account(&store, thread_id, account)?;
    let at = chrono::Utc::now().to_rfc3339();
    match store.correct(&account, thread_id, &c, &at)? {
        None => bail!("no such thread in the triage store: {thread_id}"),
        Some(made) if made.is_empty() => {
            println!("nothing changed — the verdict already said that.");
        }
        Some(made) => {
            println!("corrected {} field(s) on {thread_id}:", made.len());
            for m in &made {
                println!("  {}: {} → {}", m.field, m.was, m.now);
            }
        }
    }
    Ok(())
}

/// The account a thread lives in: what was asked for, or the only one that
/// holds it. Thread ids are account-scoped, so guessing wrong would correct a
/// different thread.
fn resolve_account(store: &TriageStore, thread_id: &str, given: Option<&str>) -> Result<String> {
    if let Some(a) = given {
        return Ok(a.to_string());
    }
    let hits: Vec<Record> = store
        .list()?
        .into_iter()
        .filter(|r| r.thread_id == thread_id)
        .collect();
    match hits.len() {
        0 => bail!("no such thread in the triage store: {thread_id}"),
        1 => Ok(hits[0].account.clone()),
        _ => bail!(
            "thread id is in several accounts ({}) — pass --account",
            hits.iter()
                .map(|r| r.account.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Score the live store against a corpus window.
///
/// Two kinds of evidence, reported apart because they are not the same claim.
///
/// **A reply is behaviour**, and it is one-sided in the way §3 of
/// `MAIL-CORPUS-RESEARCH.md` describes: answering proves the thread mattered,
/// silence proves nothing.
///
/// **A correction is testimony** — the user saying outright that a field was
/// wrong. It is the stronger signal and it is symmetric, so it is the one a
/// `triage` rule should ultimately be judged on. It is reported separately
/// rather than folded in, because merging a hundred behavioural samples with
/// three explicit corrections would let volume drown the better evidence.
fn score(account: &str, min_age_hours: i64, json_out: bool) -> Result<()> {
    let store = TriageStore::open(TriageStore::default_root()?)?;
    let records: Vec<Record> = store
        .list()?
        .into_iter()
        .filter(|r| r.account == account && r.verdict.is_some() && r.state != DISMISSED)
        .collect();
    if records.is_empty() {
        bail!("no classified threads for account `{account}` in the triage store");
    }

    let path = mecha_core::work::mecha_home()?
        .join("mail-corpus")
        .join(format!("{account}.jsonl"));
    let me = guess_self(&path)?;
    let corpus = corpus_threads(&path, &me)?;
    let by_id: std::collections::HashMap<&str, &CorpusThread> = corpus
        .iter()
        .map(|t| (t.input.thread_id.as_str(), t))
        .collect();

    let cutoff = chrono::Utc::now() - chrono::Duration::hours(min_age_hours);
    let mut graded = Vec::new();
    let mut unseen = 0usize;
    let mut too_young = 0usize;
    for r in &records {
        // Too recent to have an outcome. Excluded rather than counted as
        // unanswered: the passage of time is not evidence about a verdict.
        let settled = chrono::DateTime::parse_from_rfc3339(&r.date)
            .map(|d| d.with_timezone(&chrono::Utc) <= cutoff)
            .unwrap_or(true);
        if !settled {
            too_young += 1;
            continue;
        }
        match by_id.get(r.thread_id.as_str()) {
            // Not in the window: no evidence either way. Counting it as
            // unanswered would manufacture ground truth out of a gap in the
            // corpus, which is the failure this whole file is careful about.
            None => unseen += 1,
            Some(t) => graded.push(Graded {
                replied: t.replied,
                verdict: r.verdict.clone(),
                prefiltered: None,
            }),
        }
    }

    let corrected: Vec<&Record> = records
        .iter()
        .filter(|r| !r.corrections.is_empty())
        .collect();
    let mut by_field: std::collections::BTreeMap<&str, usize> = Default::default();
    for r in &corrected {
        for c in &r.corrections {
            *by_field.entry(c.field.as_str()).or_default() += 1;
        }
    }

    let s = Scorecard::of(&graded);
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "account": account,
                "records": records.len(),
                "with_reply_evidence": graded.len(),
                "outside_corpus_window": unseen,
                "too_young_to_score": too_young,
                "min_age_hours": min_age_hours,
                "answered": {"n": s.replied, "buried": s.replied_final_ignore,
                             "false_ignore_rate": s.false_ignore_rate()},
                "unanswered": {"n": s.unreplied, "surfaced": s.unreplied_surfaced,
                               "respond": s.unreplied_buckets[0]},
                "corrections": {"threads": corrected.len(),
                                "by_field": by_field},
                "caveat": Scorecard::caveat(),
            }))?
        );
        return Ok(());
    }

    println!(
        "triage store · {account} · {} classified thread(s)",
        records.len()
    );
    println!(
        "  scored {} · {too_young} too recent (<{min_age_hours}h, no outcome yet) \
         · {unseen} outside the corpus window (refresh with `mecha-mail corpus`)",
        graded.len()
    );
    println!("\n── behaviour: did a reply go out ──");
    println!("  answered:   {}", s.replied);
    match s.false_ignore_rate() {
        Some(r) => println!(
            "    buried as `ignore`: {} ({:.1}%)",
            s.replied_final_ignore,
            100.0 * r
        ),
        None => println!("    buried as `ignore`: n/a — no answered threads in the window"),
    }
    println!("  unanswered: {}", s.unreplied);
    println!(
        "    `respond`: {} — what day two would surface",
        s.unreplied_buckets[0]
    );
    println!("  {}", Scorecard::caveat());

    println!("\n── testimony: what you said was wrong ──");
    if corrected.is_empty() {
        println!("  no corrections yet — `mecha mail correct <thread>` records one.");
        println!("  This is the stronger signal and the one a triage rule should be judged on.");
    } else {
        println!("  {} thread(s) corrected", corrected.len());
        for (f, n) in &by_field {
            println!("    {f}: {n}");
        }
    }
    Ok(())
}

/// Walk unmined corrections through the reflector into the learning store.
async fn reflect(global: &GlobalOpts, account: Option<&str>, dry_run: bool) -> Result<()> {
    let store = TriageStore::open(TriageStore::default_root()?)?;
    let learning = mecha_core::learning::LearningStore::open(
        mecha_core::learning::LearningStore::default_root()?,
    )?;
    let mined = learning.mined_corrections()?;

    let mut todo: Vec<(Record, mecha_core::mail_triage::Correction)> = Vec::new();
    for r in store.list()? {
        if account.is_some_and(|a| a != r.account) {
            continue;
        }
        for c in &r.corrections {
            if !mined.contains(&mecha_core::mail_triage::correction_key(
                &r.account,
                &r.thread_id,
                c,
            )) {
                todo.push((r.clone(), c.clone()));
            }
        }
    }
    println!("{} correction(s) to reflect on", todo.len());
    if todo.is_empty() {
        println!("  `mecha mail correct <thread>` records one.");
        return Ok(());
    }
    if dry_run {
        for (r, c) in &todo {
            println!(
                "  would reflect: {} — {}: {} → {}",
                r.subject, c.field, c.was, c.now
            );
        }
        return Ok(());
    }

    // A provider and nothing else, like the classifier: a reflector with a
    // tool surface is a reflector that can be talked into using it, and the
    // mail it reads is the least trusted input in the system.
    let cwd = std::env::current_dir()?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| provider.default_model().to_string());
    eprintln!("reflecting with {model} ({provider_name})");

    let (mut learned, mut declined, mut failed) = (0u32, 0u32, 0u32);
    for (r, c) in todo {
        let prompt = mecha_core::mail_triage::correction_reflector_prompt(
            &r,
            &c,
            &mecha_core::mail_triage::reflector_context(&r),
        );
        let request = mecha_core::message::CompletionRequest {
            model: model.clone(),
            system: None,
            messages: vec![mecha_core::message::Message::user(prompt)],
            tools: Vec::new(),
            max_tokens: 2048,
            effort: None,
            thinking: false,
            cache_prompt: false,
        };
        let key = mecha_core::mail_triage::correction_key(&r.account, &r.thread_id, &c);
        match provider.complete(&request, None).await {
            Err(e) => {
                eprintln!("  ! {} — {e}", r.thread_id);
                failed += 1;
                // Deliberately not marked mined: a transient provider failure
                // must not bury a correction, which is the same bug the
                // classify sweep had with `failed` records.
                continue;
            }
            Ok(resp) => match mecha_core::mail_triage::parse_lesson(&resp.message.text()) {
                Err(e) => {
                    eprintln!("  ! {} — {e:#}", r.thread_id);
                    failed += 1;
                    continue;
                }
                Ok(None) => {
                    declined += 1;
                    learning.mark_correction_mined(&key)?;
                }
                Ok(Some(lesson)) => {
                    let refl = mecha_core::learning::Reflexion {
                        id: format!("triage-{key}"),
                        domain: mecha_core::learning::TRIAGE_DOMAIN.to_string(),
                        // The thread is the session here: there is no
                        // conversation, and this is what a later reader would
                        // need to find the evidence again.
                        session_id: format!("{}/{}", r.account, r.thread_id),
                        trigger: "correction".into(),
                        context: format!(
                            "classifier said {} on mail from {}",
                            r.verdict.as_ref().map(|v| v.bucket.as_str()).unwrap_or("?"),
                            r.from
                        ),
                        intervention: format!("{}: {} → {}", c.field, c.was, c.now),
                        reflexion_text: lesson.clone(),
                        error_type: Some(c.field.clone()),
                        confidence: None,
                        is_processed: false,
                        leap_run_id: None,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        // **Honest, not convenient.** This lesson was argued
                        // from mail, so it is untrusted; `learnable()` admits
                        // it because triage rules reach only the classifier,
                        // not because the origin was laundered.
                        origin: mecha_core::learning::Origin::Untrusted,
                    };
                    learning.append_reflexion(&refl)?;
                    learning.mark_correction_mined(&key)?;
                    learned += 1;
                    println!("  + {lesson}");
                }
            },
        }
    }
    println!("\n{learned} lesson(s), {declined} declined, {failed} failed");
    if learned > 0 {
        println!("`mecha learn --domain triage` consolidates them into rules.");
    }
    Ok(())
}

/// Put a thread on the task board, carrying its deadline.
#[allow(clippy::too_many_arguments)]
async fn task(
    global: &GlobalOpts,
    thread_id: &str,
    account: Option<&str>,
    name: Option<&str>,
    due: Option<&str>,
    context: &str,
    project: Option<&str>,
) -> Result<()> {
    let store = TriageStore::open(TriageStore::default_root()?)?;
    let thread_id = &resolve_thread(&store, thread_id)?;
    let account = resolve_account(&store, thread_id, account)?;
    let rec = store
        .get(&account, thread_id)
        .with_context(|| format!("no such thread in the triage store: {thread_id}"))?;

    // The classifier's summary describes the mail; a task wants an action.
    // Defaulting to it beats refusing, and the flag exists because the two are
    // genuinely different sentences.
    let name = name
        .map(str::to_string)
        .or_else(|| rec.verdict.as_ref().map(|v| v.one_line.clone()))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| rec.subject.clone());
    // **This is the phase's whole point.** A deadline the classifier already
    // found, carried without anyone re-reading the thread to find it again.
    let due = due
        .map(str::to_string)
        .or_else(|| rec.verdict.as_ref().and_then(|v| v.deadline.clone()));

    let prepared = setup::prepare_tools(global, false).await?;
    let create = find_tool(&prepared.registry, "kg_task_create")
        .context("no knowledge-graph server in this configuration — is `[[mcp]]` enabled?")?;
    let ctx = tool_ctx(&prepared);

    let mut args = json!({ "name": name, "context": context });
    if let Some(d) = &due {
        args["due"] = json!(d);
    }
    // Passed through untouched. `kg_task_create` requires it to resolve to an
    // existing node, and inventing one from a subject line would produce a
    // board that cannot be queried — the failure the project field exists to
    // prevent.
    if let Some(p) = project {
        args["project"] = json!(p);
    }
    let out = create.call(args, &ctx).await?;
    if out.is_error {
        bail!("creating the task failed: {}", out.content);
    }
    println!("{}", out.content.trim());

    store.mark(&account, thread_id, "task", mecha_core::mail_triage::ACTED)?;
    println!("\n{}", name);
    match &due {
        Some(d) => println!(
            "  due {d} (from the {})",
            if due_came_from_verdict(&rec, d) {
                "verdict"
            } else {
                "flag"
            }
        ),
        None => println!("  no due date — the classifier found none and none was given"),
    }
    println!("  context {context}");
    println!(
        "  thread {thread_id} · `mecha mail show {thread_id} --account {account}` to re-read it"
    );
    Ok(())
}

/// Whether the due date came from the classifier rather than the flag — the
/// difference between "it noticed" and "you told it", which is the thing worth
/// reporting back.
fn due_came_from_verdict(rec: &Record, due: &str) -> bool {
    rec.verdict
        .as_ref()
        .and_then(|v| v.deadline.as_deref())
        .is_some_and(|d| d == due)
}

/// Park a thread until somebody answers.
fn needs_info(thread_id: &str, account: Option<&str>, missing: &str) -> Result<()> {
    if missing.trim().is_empty() {
        bail!("say what is missing — parking a thread without naming what it waits for is dismissing it slowly");
    }
    let store = TriageStore::open(TriageStore::default_root()?)?;
    let thread_id = &resolve_thread(&store, thread_id)?;
    let account = resolve_account(&store, thread_id, account)?;
    let mut rec = store
        .get(&account, thread_id)
        .with_context(|| format!("no such thread in the triage store: {thread_id}"))?;

    rec.state = mecha_core::mail_triage::PARKED.to_string();
    rec.acted = Some("needs-info".into());
    rec.acted_at = Some(chrono::Utc::now().to_rfc3339());
    // Kept in `rest` rather than as a typed field: what a thread waits for is
    // the user's own prose, it is read by people rather than by code, and the
    // store preserves unknown keys on write.
    rec.rest.insert(
        mecha_core::mail_triage::PARKED_FOR.to_string(),
        json!(missing),
    );
    store.put(&rec)?;

    println!("parked {thread_id}");
    println!("  waiting for: {missing}");
    println!("  still yours — `mecha mail list --all` shows it; dismiss drops it instead");
    Ok(())
}

/// Turn whatever a person typed into a real thread id.
///
/// Briefings print an eight-character handle rather than a seventy-six
/// character id, so every verb that takes a thread has to accept one back.
fn resolve_thread(store: &TriageStore, given: &str) -> Result<String> {
    let ids: Vec<String> = store.list()?.into_iter().map(|r| r.thread_id).collect();
    mecha_core::mail_triage::resolve_thread_id(given, ids.iter().map(String::as_str))?
        .with_context(|| format!("no thread in the triage store matches `{given}`"))
}

/// As [`resolve_thread`], but tolerating an id the store has never seen — a
/// verb may legitimately be handed one from a search. **Ambiguity still
/// fails**: passing an ambiguous handle through would ask the provider to
/// explain it, and it answers `400 ErrorInvalidIdMalformed`.
fn resolve_thread_lenient(store: &TriageStore, given: &str) -> Result<String> {
    let ids: Vec<String> = store.list()?.into_iter().map(|r| r.thread_id).collect();
    Ok(
        mecha_core::mail_triage::resolve_thread_id(given, ids.iter().map(String::as_str))?
            .unwrap_or_else(|| given.to_string()),
    )
}

/// Act on a thread in the user's own mailbox.
///
/// `mail_triage` reaches nobody: no third party learns anything, which is why
/// it is `destructiveHint` alone rather than `external_send`, and why it is
/// never outbox-routed — staging it would make triage circular, reviewing a
/// queue in order to fill another queue.
async fn triage(
    global: &GlobalOpts,
    thread_id: &str,
    account: Option<&str>,
    action: &str,
) -> Result<()> {
    let store = TriageStore::open(TriageStore::default_root()?)?;
    let thread_id = &resolve_thread(&store, thread_id)?;
    let account = resolve_account(&store, thread_id, account)?;

    let prepared = setup::prepare_tools(global, false).await?;
    let tool = find_tool(&prepared.registry, "mail_triage")
        .context("no mail server in this configuration — is `[[mcp]]` for mecha-mail enabled?")?;
    let out = tool
        .call(
            json!({ "thread_id": thread_id, "account": account, "action": action }),
            &tool_ctx(&prepared),
        )
        .await?;
    if out.is_error {
        bail!("{action} failed: {}", out.content);
    }
    // Recorded before reporting: the mailbox has already changed, and a store
    // that disagrees with it would send the next sweep back over a thread the
    // user has dealt with.
    store.mark(&account, thread_id, action, mecha_core::mail_triage::ACTED)?;
    println!("{action}d {} — {}", handle(thread_id), out.content.trim());
    Ok(())
}

/// Parse a closed-vocabulary value, listing the alternatives on failure.
///
/// A typo must fail at the keyboard rather than write a verdict nobody asked
/// for — the same reason `mecha trigger add` validates a cron expression up
/// front instead of at three in the morning.
fn one_of<T: Copy>(name: &str, given: &str, table: &[(&str, T)]) -> Result<T> {
    table
        .iter()
        .find(|(k, _)| *k == given)
        .map(|(_, v)| *v)
        .with_context(|| {
            format!(
                "unknown {name} `{given}` — one of: {}",
                table.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
            )
        })
}

#[cfg(test)]
mod classify_exit_tests {
    use super::run_accomplished_nothing;

    /// 2026-08-19: the nightly classified 0 of 16 and systemd logged SUCCESS,
    /// because the command returned `Ok(())` whatever happened. Every check
    /// downstream — `OnFailure=`, `systemctl --failed`, doctor's failed-unit
    /// scan — reads a unit's exit code, so a broken nightly was invisible to
    /// all of them at once.
    #[test]
    fn a_run_that_did_nothing_fails_and_a_partial_one_does_not() {
        assert!(run_accomplished_nothing(0, 0, 16), "the incident");

        // Partial failure is a working nightly. Failing the unit here would
        // train someone to ignore the alarm, which costs more than it buys.
        assert!(!run_accomplished_nothing(14, 0, 2));
        assert!(!run_accomplished_nothing(1, 0, 99));

        // Pre-filter disposal is work. A sweep of nothing but bulk mail did
        // its job without one model call.
        assert!(!run_accomplished_nothing(0, 12, 0));
        assert!(!run_accomplished_nothing(0, 12, 3));

        // Nothing to do is not a failure — the common case for a nightly that
        // already swept an hour ago, and the one false alarm to avoid.
        assert!(!run_accomplished_nothing(0, 0, 0));
    }
}
