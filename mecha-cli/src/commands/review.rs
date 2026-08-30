//! `mecha review` — one place to see everything waiting on a human, and to
//! decide the knowledge graph's half of it.
//!
//! Five stores accumulate work for the owner and each grew its own verb:
//! `mecha outbox`, `mecha frontdoor`, `mecha proposals`, `mecha harness`, and
//! — in another repository entirely — the graph's merge queue. Knowing what is
//! waiting meant remembering five commands, which is how a queue reaches 6,434
//! items without anybody deciding to let it.
//!
//! This is the aggregator, and it is doctor's shape rather than a sixth store:
//! it reads what the others own and holds nothing of its own. `queues` is the
//! summary; the graph verbs below are here because the graph is the one queue
//! mecha could not previously touch at all.
//!
//! ## Why this one command shells out, when `mecha tasks` does not
//!
//! `mecha tasks` reaches the graph through the MCP tool surface, and says why:
//! reaching past the tools into the SQLite file would be a second
//! implementation of a schema owned by another repository. That argument still
//! holds, and this module does not break it — it never opens the database.
//!
//! But the tool surface **cannot accept a fact candidate**, and that is a
//! decision rather than a gap. `kg_pending` reads and `kg_verdict` files an
//! opinion that decides nothing; there is deliberately no `kg_accept`, because
//! every MCP tool lands in the model's registry, and a model that can accept
//! candidates can accept the ones its own extractor proposed.
//! `ladder.rs` states the rule this protects: *a lane must not promote itself.*
//!
//! So the decision is driven the way a person drives it — by running the
//! owner's own `mecha-graph` binary as a child process, the `/triggers` rule
//! one repository over. Nothing new becomes reachable from a prompt: the
//! model's surface is unchanged, and the only new capability belongs to
//! whoever is at the keyboard.
//!
//! The cost is honest and worth stating: `mecha` now has a *runtime, optional*
//! dependency on `mecha-graph` being installed. Optional is load-bearing —
//! every verb here degrades to a named error rather than a failure, and
//! `queues` still reports the four mecha-owned stores when the graph binary is
//! missing. A summary that vanished because one of five stores was unreachable
//! would be worse than one that says so.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use mecha_core::frontdoor::{self, Frontdoor};
use mecha_core::learning::LearningStore;
use mecha_core::outbox::OutboxStore;
use mecha_core::questions::QuestionStore;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// What is waiting, across every store (default).
    Queues {
        #[arg(long)]
        json: bool,
    },
    /// Pending graph fact candidates.
    List {
        /// Only this proposing mechanism, e.g. `llm`, `bee:suggested`.
        #[arg(long)]
        proposer: Option<String>,
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// The graph queue rolled up by proposing mechanism, with each one's
    /// human accept rate — which automated system is proposing well.
    Proposers {
        #[arg(long)]
        json: bool,
    },
    /// The graph's surfaced-verdict queue (review-on-use): live shadow
    /// facts that are about to matter — contradicting a reviewed fact,
    /// actually served in a context pack, or spot-checked by a sampled
    /// class. Listing is the default; --confirm/--refute decide one.
    ///
    /// The verbs run the owner's own `mecha-graph` binary, like every
    /// decision in this module: shadow verdicts are deliberately absent
    /// from the MCP tool surface (`kg_shadow_queue` is read-only), so the
    /// only path to a verdict runs through whoever is at the keyboard.
    Shadow {
        /// Confirm a shadow fact by uid — a human stands behind it.
        /// Conflicts with --refute: two verdicts in one line is refused
        /// rather than silently half-done, the module's standing rule.
        #[arg(long, value_name = "FACT_UID", conflicts_with = "refute")]
        confirm: Option<String>,
        /// Refute a shadow fact by uid — it was never true.
        #[arg(long, value_name = "FACT_UID")]
        refute: Option<String>,
        /// Why, for --refute; it feeds the graph's rejection memory.
        #[arg(long, requires = "refute")]
        reason: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long, conflicts_with_all = ["confirm", "refute"])]
        json: bool,
    },
    /// Individual candidates from one class, drawn at random.
    ///
    /// **The default way to look at items, and the reason is the whole
    /// point.** The queue has an order, every order it could have is
    /// correlated with something, and judging the first dozen then reading
    /// the result as the class's accept rate measures the order instead of
    /// the class. A random draw is the only selection that makes those
    /// verdicts evidence about the class — which is what the queue is short
    /// of: 40.5% of it sits in classes nobody has judged once.
    Sample {
        #[arg(long)]
        proposer: Option<String>,
        /// The cluster key — a bare predicate, or `(commitment)`.
        #[arg(long)]
        predicate: Option<String>,
        #[arg(long, short = 'n', default_value_t = 12)]
        count: usize,
        /// Redraw an earlier sample. Omit and one is drawn and printed.
        #[arg(long)]
        seed: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    /// Individual candidates in queue order — the whole class, not a sample.
    ///
    /// Here for completeness and for working a class you have already
    /// decided to clear. Do not read verdicts collected this way as a rate:
    /// see `sample`.
    Items {
        #[arg(long)]
        proposer: Option<String>,
        #[arg(long)]
        predicate: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Only these candidate ids, comma-separated, in the order given —
        /// how a similarity group's members are read in full.
        #[arg(long)]
        ids: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// One class's pending candidates grouped by semantic similarity,
    /// largest groups first — where one verdict fans out furthest.
    ///
    /// The group's face is its leader statement plus samples, never a
    /// model-written summary: the reviewable object is a real member of
    /// the group, on the outbox's rule that approving a paraphrase is
    /// approving unread.
    Groups {
        /// Required for the class view; with --all, an optional filter.
        #[arg(long)]
        proposer: Option<String>,
        /// The cluster key — a bare predicate. Commitments do not group.
        /// Meaningless with --all, which has no class.
        #[arg(long, conflicts_with = "all")]
        predicate: Option<String>,
        /// The top layer: group the WHOLE pending queue regardless of
        /// class, at the graph's stricter global floor. Every group names
        /// the classes it spans — the blast radius is part of the
        /// reviewable object — and a verdict on one rides
        /// `accept|reject --cascade <ids> --across-classes`.
        #[arg(long)]
        all: bool,
        /// Cosine floor; omitted, the graph's calibrated default applies
        /// (per mode — the global floor is stricter than the class one).
        #[arg(long)]
        threshold: Option<f64>,
        #[arg(long)]
        json: bool,
    },
    /// Rebind a candidate's unresolvable subject to a real entity — the way
    /// through `cannot resolve subject 'X'`, the commonest accept failure.
    /// The old spelling is learned as an alias, so the next candidate
    /// carrying it resolves on its own.
    Bind {
        id: i64,
        /// Exact display name of the target (else: the graph's top suggestion).
        #[arg(long)]
        to: Option<String>,
    },
    /// Accept graph fact candidates, by id or by class.
    Accept {
        ids: Vec<i64>,
        /// Bulk: every pending candidate from this proposer. Substring, as
        /// the graph matches it — pair it with `--predicate`.
        #[arg(long)]
        proposer: Option<String>,
        /// Bulk: exact match on the payload predicate. **Not** the cluster
        /// key: `(commitment)` classes have no `predicate` field and cannot
        /// be verdicted in bulk at all, by design — they materialize tasks.
        #[arg(long)]
        predicate: Option<String>,
        /// Cap on how many a bulk filter may take. The graph defaults to
        /// 500; passing it explicitly is how a caller learns it was capped.
        #[arg(long)]
        limit: Option<usize>,
        /// A subject the graph does not know becomes a new topic node
        /// instead of a failure.
        #[arg(long)]
        create_subjects: bool,
        /// What a bulk filter would hit, changing nothing. The proposer
        /// filter is a *substring* on the graph's side, so what a class
        /// verdict actually covers is worth seeing before it is applied.
        #[arg(long)]
        dry_run: bool,
        /// Cascade: the one named id is YOUR verdict; every same-class
        /// candidate semantically similar to it follows as a machine
        /// cascade the ladder never counts.
        #[arg(long, conflicts_with_all = ["proposer", "predicate"])]
        like: bool,
        /// Cosine floor for --like; omitted, the graph's default applies.
        #[arg(long)]
        threshold: Option<f64>,
        /// Cascade over an explicit member list (comma-separated ids from a
        /// groups listing) instead of re-deriving similarity.
        #[arg(long, conflicts_with_all = ["like", "proposer", "predicate"])]
        cascade: Option<String>,
        /// With --cascade: the listed ids may come from other classes —
        /// pair with a listing from `groups --all`.
        #[arg(long, requires = "cascade")]
        across_classes: bool,
    },
    /// Reject graph fact candidates, by id or by class.
    Reject {
        ids: Vec<i64>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        proposer: Option<String>,
        #[arg(long)]
        predicate: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        dry_run: bool,
        /// Cascade — see `accept --like`.
        #[arg(long, conflicts_with_all = ["proposer", "predicate"])]
        like: bool,
        /// Cosine floor for --like; omitted, the graph's default applies.
        #[arg(long)]
        threshold: Option<f64>,
        /// Cascade over an explicit member list — see `accept --cascade`.
        #[arg(long, conflicts_with_all = ["like", "proposer", "predicate"])]
        cascade: Option<String>,
        /// With --cascade: the listed ids may come from other classes —
        /// see `accept --across-classes`.
        #[arg(long, requires = "cascade")]
        across_classes: bool,
    },
}

pub async fn execute(args: Args) -> Result<()> {
    match args.cmd.unwrap_or(Cmd::Queues { json: false }) {
        Cmd::Queues { json } => queues(json),
        Cmd::List {
            proposer,
            limit,
            json,
        } => list(proposer.as_deref(), limit, json),
        Cmd::Proposers { json } => proposers(json),
        Cmd::Shadow {
            confirm,
            refute,
            reason,
            limit,
            json,
        } => shadow(
            confirm.as_deref(),
            refute.as_deref(),
            reason.as_deref(),
            limit,
            json,
        ),
        Cmd::Sample {
            proposer,
            predicate,
            count,
            seed,
            json,
        } => items(
            proposer.as_deref(),
            predicate.as_deref(),
            Draw::Sample { count, seed },
            json,
        ),
        Cmd::Items {
            proposer,
            predicate,
            limit,
            ids,
            json,
        } => items(
            proposer.as_deref(),
            predicate.as_deref(),
            match ids {
                Some(ids) => Draw::Ids { ids },
                None => Draw::Head { limit },
            },
            json,
        ),
        Cmd::Groups {
            proposer,
            predicate,
            all,
            threshold,
            json,
        } => {
            if all {
                groups_all(proposer.as_deref(), threshold, json)
            } else {
                let (Some(p), Some(key)) = (proposer.as_deref(), predicate.as_deref()) else {
                    bail!("--proposer and --predicate name the class; --all is the cross-class top layer");
                };
                groups(p, key, threshold, json)
            }
        }
        Cmd::Bind { id, to } => {
            let id_s = id.to_string();
            let mut args = vec!["bind", id_s.as_str()];
            if let Some(t) = &to {
                args.push("--to");
                args.push(t);
            }
            print!("{}", graph_cli(&args)?);
            Ok(())
        }
        Cmd::Accept {
            ids,
            proposer,
            predicate,
            limit,
            create_subjects,
            dry_run,
            like,
            threshold,
            cascade,
            across_classes,
        } => decide(
            "accept",
            &ids,
            None,
            proposer.as_deref(),
            predicate.as_deref(),
            limit,
            create_subjects,
            dry_run,
            match (&cascade, like) {
                (Some(csv), _) if across_classes => Fan::IdsAcross(csv),
                (Some(csv), _) => Fan::Ids(csv),
                (None, true) => Fan::Similar(threshold),
                (None, false) => Fan::None,
            },
        ),
        Cmd::Reject {
            ids,
            reason,
            proposer,
            predicate,
            limit,
            dry_run,
            like,
            threshold,
            cascade,
            across_classes,
        } => decide(
            "reject",
            &ids,
            reason.as_deref(),
            proposer.as_deref(),
            predicate.as_deref(),
            limit,
            false,
            dry_run,
            match (&cascade, like) {
                (Some(csv), _) if across_classes => Fan::IdsAcross(csv),
                (Some(csv), _) => Fan::Ids(csv),
                (None, true) => Fan::Similar(threshold),
                (None, false) => Fan::None,
            },
        ),
    }
}

/// The surfaced-verdict queue: list by default, decide with a flag. All
/// three shapes pass through the graph binary untouched — the graph owns
/// the rendering, this module owns only the reach.
fn shadow(
    confirm: Option<&str>,
    refute: Option<&str>,
    reason: Option<&str>,
    limit: usize,
    json: bool,
) -> Result<()> {
    if let Some(uid) = confirm {
        print!("{}", graph_cli(&["shadow", "--confirm", uid])?);
        return Ok(());
    }
    if let Some(uid) = refute {
        let mut args = vec!["shadow", "--refute", uid];
        if let Some(r) = reason {
            args.push("--reason");
            args.push(r);
        }
        print!("{}", graph_cli(&args)?);
        return Ok(());
    }
    let limit_s = limit.to_string();
    let mut args = vec!["shadow", "--limit", limit_s.as_str()];
    if json {
        args.push("--json");
    }
    print!("{}", graph_cli(&args)?);
    Ok(())
}

// ─── The reviewable-proposal stores ──────────────────────────────────────────

/// One reviewable proposal, in the shape every such store now answers in.
#[derive(Clone, serde::Serialize)]
pub(crate) struct ReviewRow {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
}

/// Where a review surface gets its rows and how it decides them. Held as argv
/// rather than as an enum of stores, on the `/triggers` rule: a surface drives
/// the command line, so nothing it can do is missing from a script.
#[derive(Clone)]
pub(crate) struct ReviewSource {
    /// Human label for the box title.
    pub label: String,
    pub list: Vec<String>,
    /// `<verb> show <id>` — the whole item. Essential rather than optional:
    /// a rule proposal's list line is "5 rule(s) from 10 reflection(s)",
    /// which is a count, not something anyone can accept on. Accepting what
    /// you cannot read is the failure the outbox's DraftView exists to
    /// prevent, and this surface had it.
    pub show: Vec<String>,
    pub accept: Vec<String>,
    pub reject: Vec<String>,
    /// True when the verb lives in `mecha-graph` rather than `mecha`.
    pub graph: bool,
}

/// How many entity proposals to ask `mecha-graph` for. Not unbounded: the
/// rows are rendered into a phone, and a store that has genuinely grown past
/// this says so by depth, which every reader shows beside the list.
pub(crate) const GRAPH_LIST_LIMIT: usize = 500;

/// The reviewable-proposal stores, and where each one's verbs live.
///
/// Keyed on the queue name [`collect_queues`] emits — one list of queues,
/// produced here and read by every surface, rather than an enum per reader
/// that has to be kept in step by hand.
///
/// It lives beside the queue names rather than inside either surface because
/// there are now **two** of them: the TUI's `/queues` modal and the web
/// review tab. Four stores answer the same shape and take the same verbs, so
/// they share one generic level in each surface rather than owning a pane
/// each — three copies of "list, show, accept, reject" would be three things
/// to keep correct, which is how `/queues` came to say "no modal for that one
/// yet" beside a count of 28, and how the web home came to print
/// `mecha harness list` on a card it could not open.
pub(crate) fn review_source(queue: &str) -> Option<ReviewSource> {
    let owned = |label: &str, verb: &[&str], graph: bool| {
        let v: Vec<String> = verb.iter().map(|s| s.to_string()).collect();
        let mut list = v.clone();
        list.extend(["list".into(), "--json".into()]);
        let mut show = v.clone();
        show.push("show".into());
        let mut accept = v.clone();
        accept.push("accept".into());
        let mut reject = v;
        reject.push("reject".into());
        Some(ReviewSource {
            label: label.to_string(),
            list,
            show,
            accept,
            reject,
            graph,
        })
    };
    match queue {
        // `mecha-graph proposals list` defaults to `--limit 20`, and the
        // depth beside it comes from `proposals summary`, which counts every
        // pending row. Left alone, the surface says 45 and shows 20 with
        // nothing admitting the cut — the exact "the count and the list
        // disagree" failure this queue was surfaced to end. Ask for more than
        // any real backlog, and let the reader compare the two numbers for
        // the case where even that is not enough.
        "graph entities" => owned("entity proposals", &["proposals"], true).map(|mut s| {
            s.list
                .extend(["--limit".into(), GRAPH_LIST_LIMIT.to_string()]);
            s
        }),
        "rule proposals" => owned("rule proposals", &["proposals"], false),
        "harness changes" => owned("harness candidates", &["harness"], false),
        // The surfaced-verdict queue rides the same generic level with
        // hand-built argv: its verdict verbs are confirm/refute (a fact was
        // never a candidate to accept), and the graph's flag spelling takes
        // the uid appended exactly where accept/reject take an id. `a` =
        // confirm, `r` = refute — one keystroke, one human verdict, the same
        // keys meaning the same commitment.
        "graph shadow" => Some(ReviewSource {
            label: "shadow verdicts".to_string(),
            list: vec!["shadow".into(), "list".into(), "--json".into()],
            show: vec!["shadow".into(), "show".into()],
            accept: vec!["shadow".into(), "--confirm".into()],
            reject: vec!["shadow".into(), "--refute".into()],
            graph: true,
        }),
        _ => None,
    }
}

/// Parse the common `list --json` shape. The graph's entity proposals
/// answer a richer object (they carry node ids and evidence); the fields
/// this needs are read by name from either, so one parser serves all three
/// stores without forcing them to a lowest common schema.
pub(crate) fn review_from_json(raw: &str) -> anyhow::Result<Vec<ReviewRow>> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let arr = v.as_array().cloned().unwrap_or_default();
    Ok(arr
        .iter()
        .map(|r| {
            let id = r["id"]
                .as_str()
                .map(str::to_string)
                .or_else(|| r["id"].as_i64().map(|n| n.to_string()))
                .unwrap_or_default();
            let kind = r["detector"]
                .as_str()
                .or_else(|| r["kind"].as_str())
                .unwrap_or("")
                .to_string();
            // The graph names the node; the others carry a title already.
            let title = r["title"].as_str().map(str::to_string).unwrap_or_else(|| {
                let subject = r["subject_name"].as_str().unwrap_or("");
                let other = r["other_name"].as_str().unwrap_or("");
                if other.is_empty() {
                    subject.to_string()
                } else {
                    format!("{subject}  +  {other}")
                }
            });
            let detail = r["evidence"]
                .as_str()
                .or_else(|| r["detail"].as_str())
                .unwrap_or("")
                .to_string();
            ReviewRow {
                id,
                kind,
                title,
                detail,
            }
        })
        .collect())
}

// ─── The graph binary ────────────────────────────────────────────────────────

/// Where `mecha-graph` lives.
///
/// `$MECHA_GRAPH_BIN` first, matching the nightly scripts that already read
/// it, then the name on `PATH`. Resolution is deliberately not cached and not
/// configured in `mecha.toml`: a project file arrives with a cloned
/// repository, and a project that could name the binary mecha runs as a child
/// process has been handed arbitrary execution — the same reasoning that keeps
/// `[[trigger]]` out of the layered config.
fn graph_bin() -> String {
    std::env::var("MECHA_GRAPH_BIN").unwrap_or_else(|_| "mecha-graph".into())
}

/// Run `mecha-graph <args>` and hand back stdout.
///
/// A missing binary is reported by name with the variable that fixes it —
/// "No such file or directory" from a child process nobody mentioned is the
/// least actionable error there is.
pub(crate) fn graph_cli(args: &[&str]) -> Result<String> {
    let bin = graph_bin();
    let out = std::process::Command::new(&bin)
        .args(args)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "`{bin}` not found — install mecha-graph, or set MECHA_GRAPH_BIN \
                     to its path. Everything else in `mecha review` works without it."
                )
            } else {
                anyhow::Error::new(e).context(format!("running {bin}"))
            }
        })?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).to_string());
    }
    // The reason may be on either stream: mecha-graph reports per-item
    // failures as `#id FAILED: …` on STDOUT and exits non-zero, while a
    // clap error or panic lands on stderr. An error that reads "failed"
    // because it looked at the empty stream is no error report at all —
    // `bind 2951` said exactly that while stdout held the whole answer.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let reason = stderr
        .trim()
        .lines()
        .next()
        .filter(|l| !l.trim().is_empty())
        .or_else(|| stdout.trim().lines().last())
        .unwrap_or("failed")
        .to_string();
    bail!("{bin} {}: {}", args.first().unwrap_or(&""), reason)
}

fn graph_json(args: &[&str]) -> Result<Value> {
    let raw = graph_cli(args)?;
    serde_json::from_str(&raw).with_context(|| {
        format!(
            "mecha-graph {} did not answer JSON",
            args.first().unwrap_or(&"")
        )
    })
}

// ─── queues ──────────────────────────────────────────────────────────────────

/// One store's backlog.
///
/// `depth` is `None` when the store could not be read — distinct from
/// `Some(0)`, because "nothing waiting" and "could not look" are opposite
/// findings and rendering them alike is how a broken reader reads as a healthy
/// queue. The same rule `sessions health` applies to a rate over no
/// denominator.
/// How long ago, in the coarsest unit that is still honest. Depth answers
/// how much is waiting; this answers how long, which is the half a queue
/// aggregator exists for — this surface was built because a queue reached
/// 6,434 items unnoticed, and depth alone cannot show a queue growing.
///
/// Coarse on purpose: "5 days" is what a person acts on, and a timestamp
/// to the second invites reading it as precision about something whose
/// only meaningful question is "longer than I meant".
fn age_of(ts: Option<&str>) -> Option<String> {
    let raw = ts?.trim();
    // Two shapes reach this: RFC3339 from mecha's own stores and SQLite's
    // "YYYY-MM-DD HH:MM:SS" from the graph. Parsing both here rather than
    // making six callers agree on one.
    let parsed = chrono::DateTime::parse_from_rfc3339(raw)
        .map(|d| d.with_timezone(&chrono::Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                .map(|n| n.and_utc())
                .ok()
        })?;
    let mins = (chrono::Utc::now() - parsed).num_minutes().max(0);
    Some(match mins {
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m if m < 60 * 24 * 60 => format!("{}d", m / (60 * 24)),
        m => format!("{}mo", m / (60 * 24 * 30)),
    })
}

/// The oldest of a set of timestamps, as an age.
fn oldest_age<'a>(stamps: impl IntoIterator<Item = &'a str>) -> Option<String> {
    age_of(stamps.into_iter().min())
}

struct Queue {
    name: &'static str,
    depth: Option<usize>,
    detail: String,
    /// The verb that opens it, for the line under the table.
    opens: &'static str,
    /// How long the oldest still-waiting item has waited. `None` when the
    /// store is empty or could not be read — an absent age, never "0m".
    oldest: Option<String>,
}

fn collect_queues() -> Vec<Queue> {
    let mut out = Vec::new();

    // The graph's merge queue, via the binary. Unreachable is a reported
    // state, never a reason to drop the other four.
    let (depth, detail, oldest) = match graph_json(&["review", "--proposers", "--json"]) {
        Ok(v) => {
            let rows = v.as_array().cloned().unwrap_or_default();
            let total: usize = rows
                .iter()
                .filter_map(|r| r["pending"].as_u64())
                .map(|n| n as usize)
                .sum();
            let unjudged: usize = rows
                .iter()
                .filter(|r| r["accept_lb"].is_null())
                .filter_map(|r| r["pending"].as_u64())
                .map(|n| n as usize)
                .sum();
            let oldest = oldest_age(rows.iter().filter_map(|r| r["oldest"].as_str()));
            (
                Some(total),
                format!(
                    "{} proposer(s); {unjudged} from mechanisms you have never judged",
                    rows.len()
                ),
                oldest,
            )
        }
        Err(e) => (None, format!("{e:#}"), None),
    };
    out.push(Queue {
        name: "graph candidates",
        depth,
        detail,
        opens: "mecha review list",
        oldest,
    });

    // The graph's *entity* proposals — a second queue in the same store,
    // and deliberately its own row rather than folded into the count above.
    // They are different work: a fact candidate is "is this true?", an
    // entity proposal is "is this the same person?", and a single number
    // covering both tells you how much is waiting without telling you what
    // kind of afternoon it is.
    let (depth, detail, oldest) = match graph_json(&["proposals", "summary", "--json"]) {
        Ok(v) => {
            let rows = v.as_array().cloned().unwrap_or_default();
            let total: usize = rows
                .iter()
                .filter_map(|r| r["pending"].as_u64())
                .map(|n| n as usize)
                .sum();
            let detectors = rows
                .iter()
                .filter(|r| r["pending"].as_u64() > Some(0))
                .count();
            let oldest = oldest_age(rows.iter().filter_map(|r| r["oldest"].as_str()));
            (
                Some(total),
                if total == 0 {
                    "nothing the entity detectors can see".to_string()
                } else {
                    format!("{detectors} detector(s) with something to say")
                },
                oldest,
            )
        }
        // An older mecha-graph has no `proposals` verb, and that reads as
        // unreadable rather than empty — the dash rule. "Nothing waiting" and
        // "could not look" are opposite findings.
        Err(e) => (None, format!("{e:#}"), None),
    };
    out.push(Queue {
        name: "graph entities",
        depth,
        detail,
        opens: "mecha-graph proposals list",
        oldest,
    });

    // The surfaced-verdict queue (review-on-use): shadow facts that are
    // about to matter. Its own row because it is the graph's NEW primary
    // review surface — since extraction mints shadow facts instead of
    // queueing, "graph candidates" above counts only what cannot become a
    // fact without a human (commitments, flags, unresolved subjects),
    // while this row is where retrieval demand asks for verdicts.
    let (depth, detail, oldest) = match graph_json(&["shadow", "--json"]) {
        Ok(v) => {
            let surfaced = v["surfaced"].as_array().cloned().unwrap_or_default();
            let live = v["shadow_live"].as_u64().unwrap_or(0);
            let served = v["shadow_served"].as_u64().unwrap_or(0);
            // The depth is the graph's pre-truncation count, never this
            // page's length — the `--top` trap, again: a capped listing
            // read as the whole queue. An older graph without the field
            // falls back to the page, which is at least a floor.
            let depth = v["surfaced_total"]
                .as_u64()
                .map(|n| n as usize)
                .unwrap_or(surfaced.len());
            // `last_served` only: it is the one stamp meaning "started
            // mattering". Mixing in `ingested_at` made a March fact that
            // surfaced this morning read as five months of waiting —
            // origin and recency are not comparable quantities, and rows
            // surfaced without a serve simply contribute no age.
            let oldest = oldest_age(surfaced.iter().filter_map(|r| r["last_served"].as_str()));
            (
                Some(depth),
                format!("{live} unreviewed facts live, {served} ever served"),
                oldest,
            )
        }
        // An older mecha-graph has no `shadow` verb — unreadable, not empty.
        Err(e) => (None, format!("{e:#}"), None),
    };
    out.push(Queue {
        name: "graph shadow",
        depth,
        detail,
        opens: "mecha review shadow",
        oldest,
    });

    let (depth, detail, oldest) = match OutboxStore::default_root().and_then(OutboxStore::open) {
        Ok(store) => match store.items() {
            Ok(items) => {
                let pending: Vec<_> = items.iter().filter(|i| i.status == "pending").collect();
                let tainted = pending
                    .iter()
                    .filter(|i| i.taint.private && i.taint.untrusted)
                    .count();
                let d = if tainted > 0 {
                    format!("{tainted} drafted with the trifecta armed")
                } else {
                    format!("{} resolved on file", items.len() - pending.len())
                };
                let oldest = oldest_age(pending.iter().map(|i| i.created_at.as_str()));
                (Some(pending.len()), d, oldest)
            }
            Err(e) => (None, format!("{e:#}"), None),
        },
        Err(e) => (None, format!("{e:#}"), None),
    };
    out.push(Queue {
        name: "outbox drafts",
        depth,
        detail,
        opens: "mecha outbox",
        oldest,
    });

    // Beside the outbox because it is its inbound twin: one queue is what a
    // run wants to send, the other is what it needs to know. A question that
    // is never answered is a delegation that never finishes, and it is
    // exactly the sort of store that reaches five figures because nothing
    // counted it — which is the incident this whole surface exists because of.
    let (depth, detail, oldest) = match QuestionStore::open_existing_default() {
        Some(store) => match store.items() {
            Ok(items) => {
                let open: Vec<_> = items.iter().filter(|q| q.is_open()).collect();
                let tainted = open.iter().filter(|q| q.taint.untrusted).count();
                let d = if open.is_empty() {
                    format!("{} answered or abandoned", items.len())
                } else if tainted > 0 {
                    format!("{tainted} asked with third-party content in the conversation")
                } else {
                    format!("{} answered or abandoned", items.len() - open.len())
                };
                let oldest = oldest_age(open.iter().map(|q| q.asked_at.as_str()));
                (Some(open.len()), d, oldest)
            }
            Err(e) => (None, format!("{e:#}"), None),
        },
        // A store that does not exist yet is genuinely empty, not unreadable.
        // The dash is for "could not look", and reporting it here would make
        // a machine that has never delegated a task indistinguishable from one
        // whose question store is broken.
        None => (Some(0), "no run has needed to ask yet".to_string(), None),
    };
    out.push(Queue {
        name: "blocked questions",
        depth,
        detail,
        opens: "mecha questions",
        oldest,
    });

    let (depth, detail, oldest) = match Frontdoor::open_default().and_then(|s| s.records()) {
        Ok(records) => {
            // Anything not closed is still somebody's problem; extraction
            // failures are called out because they wait on a human by design
            // rather than by backlog.
            let open: Vec<_> = records
                .iter()
                .filter(|r| r.state != frontdoor::CLOSED)
                .collect();
            let failed = open
                .iter()
                .filter(|r| r.state == frontdoor::EXTRACTION_FAILED)
                .count();
            let d = if failed > 0 {
                format!("{failed} whose extraction failed — those need you by design")
            } else {
                format!("{} closed", records.len() - open.len())
            };
            let oldest = oldest_age(open.iter().map(|r| r.created_at.as_str()));
            (Some(open.len()), d, oldest)
        }
        Err(e) => (None, format!("{e:#}"), None),
    };
    out.push(Queue {
        name: "front-door requests",
        depth,
        detail,
        opens: "mecha frontdoor list",
        oldest,
    });

    let (depth, detail, oldest) = match LearningStore::default_root()
        .and_then(LearningStore::open)
        .and_then(|s| s.proposals())
    {
        Ok(ps) => {
            let pending: Vec<_> = ps.iter().filter(|p| p.status == "pending").collect();
            let domains: std::collections::BTreeSet<_> =
                pending.iter().map(|p| p.domain.as_str()).collect();
            let d = if domains.is_empty() {
                format!("{} decided", ps.len())
            } else {
                format!(
                    "domains: {}",
                    domains.into_iter().collect::<Vec<_>>().join(", ")
                )
            };
            let oldest = oldest_age(pending.iter().map(|p| p.created_at.as_str()));
            (Some(pending.len()), d, oldest)
        }
        Err(e) => (None, format!("{e:#}"), None),
    };
    out.push(Queue {
        name: "rule proposals",
        depth,
        detail,
        opens: "mecha proposals",
        oldest,
    });

    // The other half of the self-improvement loop. Rule proposals come out of
    // `mecha learn`; these come out of `mecha harness ruminate` — a
    // diagnostician's proposed change to the harness itself, each carrying a
    // falsifiable prediction and a measurement it did not run.
    //
    // Called out separately rather than summed with rule proposals because
    // they are decided on different evidence and one of them is not decided
    // on evidence at all: `Security` and `Architecture` reach a person however
    // well they scored, so a count that blurred the two would hide the ones a
    // score can never clear.
    let (depth, detail, oldest) =
        match mecha_core::harness::HarnessStore::open_default().and_then(|s| s.all()) {
            Ok(cs) => {
                let pending: Vec<_> = cs.iter().filter(|c| c.pending()).collect();
                let measured = pending.iter().filter(|c| c.measurement.is_some()).count();
                let d = if pending.is_empty() {
                    format!("{} resolved", cs.len())
                } else {
                    format!(
                        "{measured} already measured; {} awaiting a run",
                        pending.len() - measured
                    )
                };
                let oldest = oldest_age(pending.iter().map(|c| c.created_at.as_str()));
                (Some(pending.len()), d, oldest)
            }
            Err(e) => (None, format!("{e:#}"), None),
        };
    out.push(Queue {
        name: "harness changes",
        depth,
        detail,
        opens: "mecha harness list",
        oldest,
    });

    out
}

fn queues(as_json: bool) -> Result<()> {
    let qs = collect_queues();
    if as_json {
        let rows: Vec<Value> = qs
            .iter()
            .map(|q| {
                json!({ "queue": q.name, "depth": q.depth, "detail": q.detail,
                        "opens": q.opens, "oldest": q.oldest })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    let total: usize = qs.iter().filter_map(|q| q.depth).sum();
    let unread = qs.iter().filter(|q| q.depth.is_none()).count();
    println!("{total} item(s) waiting on you\n");
    for q in &qs {
        match q.depth {
            // The age sits beside the count, not in the detail: it is the
            // same kind of fact as the depth — a property of waiting — and
            // burying it in prose is how it stayed invisible.
            Some(n) => println!(
                "{n:>6}  {:>6}  {:<22} {}",
                q.oldest.as_deref().unwrap_or("—"),
                q.name,
                q.detail
            ),
            // A dash, never a zero.
            None => println!(
                "{:>6}  {:>6}  {:<22} unreadable: {}",
                "—", "—", q.name, q.detail
            ),
        }
    }
    println!();
    for q in &qs {
        println!("        {:<22} {}", q.name, q.opens);
    }
    if unread > 0 {
        println!("\n{unread} store(s) could not be read — the counts above are a floor.");
    }
    Ok(())
}

// ─── the graph's half ────────────────────────────────────────────────────────

fn proposers(as_json: bool) -> Result<()> {
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&graph_json(&["review", "--proposers", "--json"])?)?
        );
        return Ok(());
    }
    print!("{}", graph_cli(&["review", "--proposers", "--text"])?);
    Ok(())
}

fn list(proposer: Option<&str>, limit: usize, as_json: bool) -> Result<()> {
    // One sample per cluster: the modal renders it, and it is the only thing
    // on the classes screen saying what a class actually contains before you
    // verdict the whole of it. `0` here made that column blank on every row.
    let v = graph_json(&["review", "--clusters", "--samples", "1", "--json"])?;
    let clusters = v.as_array().cloned().unwrap_or_default();
    let rows: Vec<&Value> = clusters
        .iter()
        .filter(|c| proposer.is_none_or(|p| c["proposed_by"].as_str() == Some(p)))
        .take(limit)
        .collect();
    if as_json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!(
            "nothing pending{}",
            match proposer {
                Some(p) => format!(" from {p}"),
                None => String::new(),
            }
        );
        return Ok(());
    }
    let total: u64 = rows.iter().filter_map(|c| c["pending"].as_u64()).sum();
    println!("{total} pending in {} class(es)\n", rows.len());
    for c in &rows {
        let (a, r) = (
            c["accepted_hist"].as_i64().unwrap_or(0),
            c["rejected_hist"].as_i64().unwrap_or(0),
        );
        // No rate without a denominator — see `queues`.
        let hist = match a + r {
            0 => "unjudged".to_string(),
            n => format!("{:.0}% of {n}", 100.0 * a as f64 / n as f64),
        };
        println!(
            "{:>6}  {} · {}  [{hist}]",
            c["pending"].as_u64().unwrap_or(0),
            c["proposed_by"].as_str().unwrap_or("?"),
            c["predicate"].as_str().unwrap_or("?"),
        );
    }
    println!("\nDecide: mecha review accept|reject <id>…  ·  browse: mecha review proposers");
    Ok(())
}

/// The top layer: the whole pending queue grouped across classes. The
/// graph does everything; this process renders — including the classes
/// each group spans, because the blast radius is part of the reviewable
/// object, and the singleton count, because a view that shows less than
/// the queue must say how much less.
fn groups_all(proposer: Option<&str>, threshold: Option<f64>, as_json: bool) -> Result<()> {
    let t_s = threshold.map(|t| t.to_string());
    let mut args: Vec<&str> = vec!["review", "--groups", "--across-classes"];
    if let Some(p) = proposer {
        args.push("--proposer");
        args.push(p);
    }
    if let Some(t) = &t_s {
        args.push("--threshold");
        args.push(t);
    }
    let answer = graph_json(&args)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&answer)?);
        return Ok(());
    }
    let rows = answer["groups"].as_array().cloned().unwrap_or_default();
    let considered = answer["considered"].as_u64().unwrap_or(0);
    if rows.is_empty() {
        println!("no cross-class groups: nothing repeats above the global threshold");
        return Ok(());
    }
    let covered: u64 = rows
        .iter()
        .map(|g| 1 + g["members"].as_array().map_or(0, |m| m.len() as u64))
        .sum();
    println!(
        "{} group(s) covering {covered} of {considered} pending (cosine >= {}; singletons stay in their class listings)\n",
        rows.len(),
        answer["threshold"].as_f64().unwrap_or(0.0),
    );
    for g in &rows {
        println!(
            "  x{:<4} #{:<7} {}",
            1 + g["members"].as_array().map_or(0, |m| m.len()),
            g["leader_id"].as_i64().unwrap_or(0),
            g["leader_statement"].as_str().unwrap_or("?"),
        );
        if let Some(classes) = g["classes"].as_object() {
            let span: Vec<String> = classes
                .iter()
                .map(|(c, n)| format!("{c} x{}", n.as_u64().unwrap_or(0)))
                .collect();
            println!("           spans: {}", span.join(", "));
        }
        for sm in g["sample"].as_array().into_iter().flatten() {
            if let Some(t) = sm.as_str() {
                println!("           ~ {t}");
            }
        }
    }
    println!("\none verdict per group: mecha review accept|reject <leader-id> --cascade <ids> --across-classes");
    Ok(())
}

/// One class's queue grouped by semantic similarity — the graph does the
/// embedding and clustering; this process only renders. Largest first, so
/// the top row is where one verdict resolves the most items.
fn groups(proposer: &str, predicate: &str, threshold: Option<f64>, as_json: bool) -> Result<()> {
    let t_s = threshold.map(|t| t.to_string());
    let mut args: Vec<&str> = vec![
        "review",
        "--groups",
        "--proposer",
        proposer,
        "--predicate",
        predicate,
    ];
    if let Some(t) = &t_s {
        args.push("--threshold");
        args.push(t);
    }
    let answer = graph_json(&args)?;
    if as_json {
        // The envelope verbatim — the threshold inside is what a TUI's
        // adjustment steps from.
        println!("{}", serde_json::to_string_pretty(&answer)?);
        return Ok(());
    }
    let rows = answer["groups"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        println!("no groups: nothing in {proposer} · {predicate} repeats above the threshold");
        return Ok(());
    }
    let covered: u64 = rows
        .iter()
        .map(|g| 1 + g["members"].as_array().map_or(0, |m| m.len() as u64))
        .sum();
    println!(
        "{} group(s) covering {covered} candidate(s) in {proposer} · {predicate}\n",
        rows.len()
    );
    for g in &rows {
        let n = 1 + g["members"].as_array().map_or(0, |m| m.len());
        println!(
            "  ×{n:<4} #{:<7} {}",
            g["leader_id"].as_i64().unwrap_or(0),
            g["leader_statement"].as_str().unwrap_or("?"),
        );
        for sm in g["sample"].as_array().into_iter().flatten() {
            println!("           ~ {}", sm.as_str().unwrap_or("?"));
        }
    }
    println!("\nOne verdict per group: mecha review accept|reject <leader-id> --like");
    Ok(())
}

/// How the items were chosen. Named rather than a bare bool, because which
/// one produced a set of verdicts decides whether those verdicts are evidence
/// about the class or only about its head.
pub enum Draw {
    Sample {
        count: usize,
        seed: Option<u64>,
    },
    Head {
        limit: usize,
    },
    /// An explicit id list — a similarity group's members. Not a sample and
    /// not the head: the set was named by the caller, so its verdicts are
    /// about exactly those items.
    Ids {
        ids: String,
    },
}

/// How many rows a named id set may return: exactly as many as were named.
///
/// Separate and tested because the failure it prevents is invisible — a cap
/// that trims a set is indistinguishable from a set that was that size.
fn ids_limit(ids: &str) -> usize {
    ids.split(',')
        .filter(|s| !s.trim().is_empty())
        .count()
        .max(1)
}

/// Individual candidates from one class.
///
/// The selection happens in `mecha-graph`, which owns the queue — pulling
/// 6,434 payloads across a pipe to pick twelve would be this process holding
/// a second, staler copy of a store it does not own.
fn items(proposer: Option<&str>, predicate: Option<&str>, draw: Draw, as_json: bool) -> Result<()> {
    let (count, seed, limit, ids) = match &draw {
        Draw::Sample { count, seed } => (Some(count.to_string()), *seed, None, None),
        Draw::Head { limit } => (None, None, Some(limit.to_string()), None),
        // A named set carries its own limit, and forgetting that was a
        // silent cap: `--top` defaults to **10** in the graph, and it bounds
        // `--ids` too, so a dive into a group of seventeen showed ten
        // members and said nothing. Not a listing that got long — a set the
        // caller enumerated, seven of which simply were not there. Every
        // surface over this inherited it: the TUI's group dive since it
        // shipped, and the phone's the day it was written.
        //
        // The rule is the one the review sampler already states out loud —
        // if coverage is bounded, say what was dropped. Here there is
        // nothing to say, because there is no reason to bound it: the count
        // is the caller's own list length.
        Draw::Ids { ids } => (
            None,
            None,
            Some(ids_limit(ids).to_string()),
            Some(ids.clone()),
        ),
    };
    let seed_s = seed.map(|s| s.to_string());
    let mut args: Vec<&str> = vec!["review"];
    if let Some(p) = proposer {
        args.push("--proposer");
        args.push(p);
    }
    if let Some(p) = predicate {
        args.push("--predicate");
        args.push(p);
    }
    if let Some(c) = &count {
        args.push("--sample");
        args.push(c);
    }
    if let Some(s) = &seed_s {
        args.push("--seed");
        args.push(s);
    }
    if let Some(l) = &limit {
        args.push("--top");
        args.push(l);
    }
    if let Some(i) = &ids {
        args.push("--ids");
        args.push(i);
    }
    if as_json {
        args.push("--json");
        println!("{}", serde_json::to_string_pretty(&graph_json(&args)?)?);
        return Ok(());
    }
    args.push("--text");
    // The child signs off with its own verbs, which are the wrong ones from
    // here — a reader who followed them would leave the surface they are
    // standing in. Only the footer is rewritten; nothing about the candidates
    // or the outcome is touched, which is the line this module holds
    // elsewhere too.
    let out = graph_cli(&args)?;
    print!(
        "{}",
        out.replace("mecha-graph accept", "mecha review accept")
            .replace("mecha-graph reject", "mecha review reject")
    );
    if matches!(draw, Draw::Head { .. }) {
        println!(
            "\nThese are the head of the queue, not a sample — verdicts collected\n\
             this way describe the ordering. Use `mecha review sample` for a rate."
        );
    }
    Ok(())
}

/// How many candidates a verdict actually landed on, read off the child's
/// own report.
///
/// The graph prints one line per candidate (`#123 accepted → fact …`), not a
/// total, so neither the last line nor the row's own `pending` answers "how
/// many". The row is the wrong source twice over: the graph's bulk proposer
/// filter is a *substring*, and `--limit` caps the set at 500 by default — so
/// a class showing 1,084 pending can report "accepted 1084" after 500 landed.
/// Counting the child's outcome lines is the only account that matches what
/// happened.
/// The cascade summary out of a `--like` report: (cascaded, left pending).
///
/// The seed's own line is `tally_report`'s to count; the fan-out is reported
/// once by the graph ("cascade: N accepted, M left pending — …") and read
/// from that line rather than re-counted here, because the child is the only
/// process that knows what it did.
pub fn cascade_tally(report: &str) -> Option<(usize, usize)> {
    let line = report
        .lines()
        .find(|l| l.trim_start().starts_with("cascade:"))?;
    let mut nums = line
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<usize>().ok());
    Some((nums.next()?, nums.next().unwrap_or(0)))
}

pub fn tally_report(report: &str) -> (usize, usize) {
    let mut done = 0;
    let mut failed = 0;
    for line in report.lines() {
        let l = line.trim();
        if !l.starts_with('#') {
            continue;
        }
        if l.contains("FAILED") {
            failed += 1;
        } else if l.contains(" accepted") || l.contains(" rejected") {
            done += 1;
        }
    }
    (done, failed)
}

/// Drive the graph's own accept/reject.
///
/// Ids are passed through untouched and the child's own report is printed
/// rather than re-worded: this process is a driver, and a driver that
/// paraphrases its child's outcome is a second account of what happened.
/// How far one verdict reaches. Named rather than a pair of flags, because
/// `like: bool` beside `cascade: Option<…>` invites the combination that
/// means nothing.
#[derive(Clone, Copy)]
pub enum Fan<'a> {
    /// Exactly the named ids or class filters.
    None,
    /// Seed plus the similar set the graph re-derives by embedding, at an
    /// optional cosine floor.
    Similar(Option<f64>),
    /// Seed plus this explicit member list (comma-separated ids) from a
    /// groups listing — the set the person actually read. No embedder runs,
    /// and the graph vets every id against the seed's class.
    Ids(&'a str),
    /// Like [`Fan::Ids`], from a `groups --all` listing: the graph's vet
    /// admits pending ids from other classes. Everything else holds — one
    /// seed, one human verdict, members labeled `cascade:<seed>`.
    IdsAcross(&'a str),
}

#[allow(clippy::too_many_arguments)]
fn decide(
    verb: &str,
    ids: &[i64],
    reason: Option<&str>,
    proposer: Option<&str>,
    predicate: Option<&str>,
    limit: Option<usize>,
    create_subjects: bool,
    dry_run: bool,
    fan: Fan,
) -> Result<()> {
    print!(
        "{}",
        decide_report(
            verb,
            ids,
            reason,
            proposer,
            predicate,
            limit,
            create_subjects,
            dry_run,
            fan,
        )?
    );
    Ok(())
}

/// Drive the graph's own accept/reject and hand back what it said.
///
/// Ids and the child's report are passed through untouched: this process is a
/// driver, and a driver that paraphrases its child's outcome is a second
/// account of what happened. The modal reads the returned text for the count
/// rather than assuming its own row was the whole match — the graph's bulk
/// proposer filter is a *substring* and its `--limit` caps the set, so the
/// number acted on is the child's to report.
#[allow(clippy::too_many_arguments)]
pub fn decide_report(
    verb: &str,
    ids: &[i64],
    reason: Option<&str>,
    proposer: Option<&str>,
    predicate: Option<&str>,
    limit: Option<usize>,
    create_subjects: bool,
    dry_run: bool,
    fan: Fan,
) -> Result<String> {
    // A cascade fans out from exactly one human verdict; a seed set would
    // make "whose verdict was this" unanswerable in the record. Checked
    // first: a fan with no id should hear about the seed, not about class
    // filters it must not combine with.
    if !matches!(fan, Fan::None) && ids.len() != 1 {
        bail!("a cascade takes exactly one candidate id (the seed)");
    }
    if ids.is_empty() && proposer.is_none() && predicate.is_none() {
        bail!("give candidate ids, or --proposer / --predicate for a whole class");
    }
    // A cluster key in parentheses is not a payload predicate. `(commitment)`
    // candidates carry `kind`, never `predicate`, so a bulk filter on it
    // matches nothing — silently, which is the worst possible answer for a
    // verdict. Refused by name instead.
    if let Some(p) = predicate {
        if p.starts_with('(') {
            bail!(
                "`{p}` is a cluster kind, not a predicate — these materialize tasks and are reviewed one at a time (mecha review sample)"
            );
        }
    }
    let id_strings: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    let limit_s = limit.map(|l| l.to_string());
    let mut args: Vec<&str> = vec![verb];
    args.extend(id_strings.iter().map(|s| s.as_str()));
    if verb == "reject" {
        if let Some(r) = reason {
            args.push("--reason");
            args.push(r);
        }
    }
    if let Some(p) = proposer {
        args.push("--proposer");
        args.push(p);
    }
    if let Some(p) = predicate {
        args.push("--predicate");
        args.push(p);
    }
    if let Some(l) = &limit_s {
        args.push("--limit");
        args.push(l);
    }
    if create_subjects {
        args.push("--create-subjects");
    }
    if dry_run {
        args.push("--dry-run");
    }
    let t_s = match fan {
        Fan::Similar(t) => t.map(|t| t.to_string()),
        _ => None,
    };
    match fan {
        Fan::None => {}
        Fan::Similar(_) => {
            args.push("--like");
            if let Some(t) = &t_s {
                args.push("--threshold");
                args.push(t);
            }
        }
        Fan::Ids(csv) => {
            args.push("--cascade");
            args.push(csv);
        }
        Fan::IdsAcross(csv) => {
            args.push("--cascade");
            args.push(csv);
            args.push("--across-classes");
        }
    }
    graph_cli(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A named set is never trimmed. `--top` defaults to 10 in the graph and
    /// bounds `--ids` as well, so before this the dive into a 17-member
    /// similarity group returned 10 rows and said nothing about the other
    /// seven — a set the caller enumerated, silently answered short.
    #[test]
    fn a_named_id_set_asks_for_exactly_as_many_as_it_names() {
        assert_eq!(ids_limit("9281"), 1);
        assert_eq!(ids_limit("9281,9286,9799"), 3);
        assert_eq!(
            ids_limit("9281,9286,9799,9800,10028,10035,10089,10679,10836,11564,9302,9310"),
            12,
            "past the default cap of ten, which is the case that was broken"
        );
        // Trailing and empty fields are not ids; the floor keeps a malformed
        // list from asking the graph for zero rows, which reads as an empty
        // group rather than as a bad request.
        assert_eq!(ids_limit("9281,"), 1);
        assert_eq!(ids_limit(""), 1);
    }

    /// The count comes off the child's report, not off the row.
    ///
    /// Found by review: the modal formatted `"accepted {n}"` from the class's
    /// own `pending`, while the graph caps a bulk filter at 500 and matches
    /// proposers by *substring* — so a 1,084-item class would report 1,084
    /// after 500 landed. A verdict that misreports its own size is the kind
    /// of number somebody quotes.
    #[test]
    fn the_tally_counts_the_childs_outcomes_not_the_rows() {
        let report = "\
#601 accepted → fact 3f2a
#602 accepted → task task-9
#603 FAILED: no pending candidate 603
#604 accepted → fact 8b1c
";
        assert_eq!(tally_report(report), (3, 1));

        let rejects = "#1 rejected\n#2 rejected\n";
        assert_eq!(tally_report(rejects), (2, 0));

        // A dry run reports a total and no per-candidate outcomes, so nothing
        // is counted as done — which is correct: nothing was.
        let dry =
            "would match #7411 [linker:knn] …\n56 candidates match (dry run — nothing changed)\n";
        assert_eq!(tally_report(dry), (0, 0));

        assert_eq!(tally_report(""), (0, 0));
    }

    /// A cluster kind is refused by name rather than passed to a filter that
    /// would match nothing.
    ///
    /// `precheck::cluster_key` returns `(commitment)` for commitments, while
    /// the graph's bulk `--predicate` reads `payload["predicate"]` — a field
    /// commitments do not have. The two conventions share a flag name, so the
    /// mismatch is silent: zero rows, no error, and a reviewer believing a
    /// class was cleared.
    #[test]
    fn a_cluster_kind_is_refused_rather_than_silently_matching_nothing() {
        let err = decide_report(
            "accept",
            &[],
            None,
            Some("llm"),
            Some("(commitment)"),
            None,
            false,
            true,
            Fan::None,
        )
        .expect_err("must refuse");
        let msg = format!("{err:#}");
        assert!(msg.contains("cluster kind"), "{msg}");
        assert!(
            msg.contains("one at a time"),
            "names the way through: {msg}"
        );
    }

    /// Neither ids nor filters is an error, never a silent no-op.
    #[test]
    fn a_verdict_with_no_target_is_refused() {
        let err = decide_report(
            "accept",
            &[],
            None,
            None,
            None,
            None,
            false,
            false,
            Fan::None,
        )
        .expect_err("must refuse");
        assert!(format!("{err:#}").contains("candidate ids"));
    }

    /// A `--like` cascade fans out from exactly one seed — a seed set would
    /// make "whose verdict was this" unanswerable in the record.
    #[test]
    fn a_cascade_takes_exactly_one_seed() {
        for ids in [vec![], vec![1, 2]] {
            for fan in [Fan::Similar(None), Fan::Ids("7,8")] {
                let err = decide_report("accept", &ids, None, None, None, None, false, false, fan)
                    .expect_err("must refuse");
                assert!(format!("{err:#}").contains("exactly one"), "{ids:?}");
            }
        }
    }

    /// Pull the strings out of a `const NAME = ['a', 'b'];` literal in a
    /// Svelte file, however it is wrapped.
    ///
    /// It reads bracket-to-bracket rather than line-by-line on purpose. The
    /// first version matched the one line the declaration sat on, which
    /// quietly made "keep this array on one line" a requirement of files this
    /// test does not own — unenforced by anything (the web app has no
    /// prettier and no eslint), undocumented at the declaration, and load
    /// bearing only inside a Rust test three directories away. A peer about
    /// to reformat `App.svelte` offered to work around it, which is what
    /// showed it up: a guard that constrains how other people may format
    /// their code has overreached, and the fix belongs in the guard.
    ///
    /// It still panics rather than returning an empty set, because an empty
    /// allowlist would make every assertion below vacuously true.
    fn js_string_array(src: &str, decl: &str) -> Vec<String> {
        let from = src
            .find(decl)
            .unwrap_or_else(|| panic!("`{decl}` is gone — this guard is reading nothing"));
        let rest = &src[from..];
        let open = rest
            .find('[')
            .unwrap_or_else(|| panic!("`{decl}` is no longer an array literal"));
        let close = open
            + rest[open..]
                .find(']')
                .unwrap_or_else(|| panic!("`{decl}` has no closing bracket"));
        let out: Vec<String> = rest[open + 1..close]
            .split(',')
            .map(|s| s.trim().trim_matches(['\'', '"']).to_string())
            .filter(|s| !s.is_empty())
            .collect();
        assert!(!out.is_empty(), "`{decl}` parsed as empty");
        out
    }

    /// The web home page renders `review queues --json` through two hardcoded
    /// maps: queue name to title, and queue name to destination. Both are
    /// readers of *this* function's output living in another language, so
    /// nothing links them — `blocked questions` was added here and the home
    /// page went on rendering it under its raw wire name with nowhere to go,
    /// while the tasks tab had been showing those very items all along.
    ///
    /// **Both halves are checked, because the first version of this test
    /// checked only titles and that was not enough.** A missing title is
    /// loud: the card shows a lowercase wire name. A wrong *destination* is
    /// silent — `navigate("reviw/outbox")` leaves the router with no matching
    /// view, so the card keeps its chevron, lands you back on home, lights no
    /// nav tab, and puts `#reviw/outbox` in the URL. Measured rather than
    /// assumed: with that typo in place, the title-only version passed.
    ///
    /// Every file is read with `include_str!`, so this cannot pass by
    /// checking a path that does not exist. It verifies *naming and
    /// reachability* only — whether a queue **should** have a page is a
    /// product judgement (three are genuinely CLI-only), and the page states
    /// that per card by printing the command that does open it.
    #[test]
    fn every_queue_the_backlog_reports_is_named_and_reachable_from_the_web_home() {
        let home = include_str!("../../../web/src/lib/Home.svelte");
        let block = |after: &str| -> String {
            home.split_once(after)
                .unwrap_or_else(|| panic!("Home.svelte must still declare `{after}`"))
                .1
                .split_once("};")
                .expect("the map must still close with `};`")
                .0
                .to_string()
        };
        let labels = block("const queueLabels");
        let targets_src = block("const queueTargets");

        // The router's own tables, so a destination is checked against what
        // actually resolves rather than against a copy of it kept here.
        let views = js_string_array(include_str!("../../../web/src/App.svelte"), "const views =");
        let review_panes = js_string_array(
            include_str!("../../../web/src/lib/Review.svelte"),
            "const panes =",
        );
        let settings_panes = js_string_array(
            include_str!("../../../web/src/lib/Settings.svelte"),
            "const PANES =",
        );

        // Every `name: "…"` in this file is a Queue row; keep it that way, or
        // this reads a literal that is not a queue.
        let names: Vec<&str> = include_str!("review.rs")
            .lines()
            .filter_map(|l| l.trim().strip_prefix("name: \""))
            .filter_map(|l| l.split_once('"'))
            .map(|(n, _)| n)
            .collect();
        // A sentinel, not just a count: a count still passes when the
        // extraction breaks and finds a different eight things.
        assert!(
            names.contains(&"outbox drafts") && names.len() >= 8,
            "the queue-row extraction found {names:?} — did the rows move?"
        );

        let targets: Vec<(&str, &str)> = targets_src
            .lines()
            .filter_map(|l| l.trim().strip_prefix('\''))
            .filter_map(|l| l.split_once("': '"))
            .filter_map(|(q, rest)| rest.split_once('\'').map(|(t, _)| (q, t)))
            .collect();
        assert!(!targets.is_empty(), "queueTargets parsed as empty");

        for name in &names {
            assert!(
                labels.contains(&format!("'{name}':")),
                "queue {name:?} has no entry in Home.svelte's queueLabels — the \
                 card renders under its wire name. Add a title, and a \
                 queueTargets entry if a page on the phone shows it."
            );
        }

        for (queue, target) in &targets {
            assert!(
                names.contains(queue),
                "queueTargets routes {queue:?}, which no longer exists — a \
                 renamed queue leaves its destination behind as dead config."
            );
            let (view, sub) = target.split_once('/').unwrap_or((target, ""));
            assert!(
                views.contains(&view.to_string()),
                "queue {queue:?} points at {target:?}, but the router knows no \
                 view named {view:?} ({views:?}) — the card keeps its chevron \
                 and silently lands on home with no nav tab lit."
            );
            let panes = match view {
                "review" => &review_panes,
                "settings" => &settings_panes,
                // `graph`'s sub-hash is a search term, not a fixed pane.
                _ => continue,
            };
            assert!(
                sub.is_empty() || panes.contains(&sub.to_string()),
                "queue {queue:?} points at {target:?}, but {view:?} has no pane \
                 {sub:?} ({panes:?}) — it opens the view's default instead, \
                 which is the wrong page arrived at quietly."
            );
        }
    }

    /// The fan-out count comes off the child's own cascade line — the only
    /// process that knows what it did — and absence reads as absence.
    #[test]
    fn the_cascade_tally_reads_the_childs_line() {
        let report = "#9281 rejected (your verdict)\n\
                      cascade: 14 rejected, 2 left pending — one human verdict on the ladder\n";
        assert_eq!(cascade_tally(report), Some((14, 2)));
        assert_eq!(cascade_tally("#9281 rejected\n"), None);
    }
}
