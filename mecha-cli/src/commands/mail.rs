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
    changed_fields, needs_body, prefilter, Graded, Record, Scorecard, ThreadInput, TriageStore,
    Verdict, BODY_CHARS_MAX, CLASSIFIED, DISMISSED, FAILED,
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
    },
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd.unwrap_or(Cmd::List {
        all: false,
        json: false,
    }) {
        Cmd::List { all, json } => list(all, json),
        Cmd::Show { thread_id, account } => show(global, &thread_id, account.as_deref()).await,
        Cmd::Classify {
            account,
            limit,
            force,
            dry_run,
        } => classify(global, account.as_deref(), limit, force, dry_run).await,
        Cmd::Dismiss { thread_id, account } => dismiss(&thread_id, account.as_deref()),
        Cmd::Eval {
            account,
            sample,
            seed,
            prefilter_only,
            json,
        } => eval(global, &account, sample, seed, prefilter_only, json).await,
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

fn list(all: bool, as_json: bool) -> Result<()> {
    let Some(store) = TriageStore::open_existing_default() else {
        println!("nothing classified yet — run `mecha mail classify`");
        return Ok(());
    };
    let rows: Vec<Record> = store
        .list()?
        .into_iter()
        .filter(|r| all || r.needs_me() || r.state == FAILED)
        .collect();

    if as_json {
        // The typed view, not the record: `list --json` is what a script or a
        // modal reads, and neither has a human's excuse for seeing the prose.
        let out: Vec<Value> = rows.iter().map(|r| r.for_privileged_run()).collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("nothing needs you");
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
    let account = match account {
        Some(a) => a.to_string(),
        None => {
            // One unambiguous match is a convenience; several is a question.
            let hits: Vec<Record> = store
                .list()?
                .into_iter()
                .filter(|r| r.thread_id == thread_id)
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
            force || !store.is_known(a, t)
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

        let verdict =
            mecha_core::mail_triage::classify(provider.as_ref(), &model, &thread, &today).await;

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
    let mut failed = 0u32;
    for (i, t) in chosen.iter().enumerate() {
        // Judged as of the day it arrived. Grading a year-old deadline against
        // today would score every one of them as passed.
        let today = t.input.date.get(..10).unwrap_or("1970-01-01");
        match mecha_core::mail_triage::classify(provider.as_ref(), &model, &t.input, today).await {
            Ok(v) => graded.push(Graded {
                replied: t.replied,
                verdict: Some(v),
                prefiltered: None,
            }),
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
                             "false_ignore_rate": s.false_ignore_rate()},
                "unanswered": {"n": s.unreplied, "surfaced": s.unreplied_surfaced},
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
    println!("\n── unanswered threads — no ground truth ──");
    println!("  graded:            {}", s.unreplied);
    println!(
        "  surfaced as needing you: {} ({:.1}%)",
        s.unreplied_surfaced,
        100.0 * s.unreplied_surfaced as f64 / s.unreplied.max(1) as f64
    );
    println!("  {}", Scorecard::caveat());
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
