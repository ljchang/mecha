//! `mecha features` — which optional parts of mecha are on, the way to turn
//! on each of the rest, and `enable` / `disable` to answer the switches
//! (`docs/FEATURES-DESIGN.md`).
//!
//! The human half of [`mecha_core::feature`]. **No network**: the core module
//! reads the config and the disk, never a server, because several servers
//! here start the moment they are asked. The switches are written in place
//! into the global `config.toml` (`feature::write_switches`), never through
//! a rewrite that would drop comments or a newer build's key.

use anyhow::{Context, Result};
use mecha_core::config::Config;
use mecha_core::feature::{self, Feature, Row, State, Switch};
use mecha_core::recommend::{self, Budget, Floor, Machine, Peak, Sum};
use mecha_core::sidecar;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
    /// Machine output: one object per feature, `state` naming which of
    /// on / off / blocked / unready / unknown it is.
    #[arg(long)]
    pub json: bool,
    /// Add up the memory of every model the shown features need — any not
    /// off or blocked, so one switched on but not yet set up counts too —
    /// against this machine's (the card's and the host's, if it has a card).
    /// Reads `/proc/meminfo` and `nvidia-smi`; asks no server.
    #[arg(long)]
    pub probe: bool,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Switch features on in the global `config.toml`'s `[features]` table.
    ///
    /// Refuses a part (`ocr`, `dictate`: turn on its parent and its setting)
    /// and a feature whose dependency is off, printing the command that
    /// switches both.
    Enable {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Switch features off in the global `config.toml`'s `[features]` table.
    /// Their settings stay, for when they are switched back on.
    Disable {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// What a feature runs beside mecha, whether this machine already has
    /// it, and every model file it would download. Read-only: nothing is
    /// installed, and no server is asked (a part plans as its parent).
    Plan {
        id: String,
        /// Machine output: the plan as one object.
        #[arg(long)]
        json: bool,
        /// Hash every model file that is not a blob named by its sha256 —
        /// a hand-placed one — instead of reporting it unverified. Reads
        /// the whole file (seconds for the 22 GB chat model).
        #[arg(long)]
        verify: bool,
    },
}

pub fn execute(args: Args) -> Result<()> {
    // Checked here, not with clap's `args_conflicts_with_subcommands`: that
    // also refuses the global flags (`mecha features --yes enable graph`).
    if args.probe && args.cmd.is_some() {
        anyhow::bail!("`--probe` reads the machine; it does not go with `enable` or `disable`");
    }
    match args.cmd {
        Some(Cmd::Enable { ids }) => return set(&ids, true),
        Some(Cmd::Disable { ids }) => return set(&ids, false),
        Some(Cmd::Plan { id, json, verify }) => return plan(&id, json, verify),
        None => {}
    }
    let home = mecha_core::work::mecha_home()?;
    // The global config only, as `setup` and `doctor` read it: which
    // features an install has is a property of the machine, and `[features]`
    // is stripped from project layers anyway.
    let cfg = Config::load_global()?;
    let facts = feature::Facts::read(&home, &cfg);
    let rows = feature::all(&facts);
    for key in feature::unknown_switches(&cfg) {
        eprintln!("mecha: `[features] {key}` is not a feature this build knows — a newer build's, or a typo; ignored");
    }
    if args.probe {
        let shown: Vec<Feature> = rows.iter().filter(|r| r.shown).map(|r| r.id).collect();
        let budget = recommend::budget(recommend::Machine::read()?, &shown);
        if args.json {
            println!("{}", serde_json::to_string_pretty(&budget)?);
        } else {
            print!("{}", render_budget(&budget));
        }
        return Ok(());
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    print!("{}", render(&rows, &feature::announcements(&facts)));
    Ok(())
}

fn set(ids: &[String], on: bool) -> Result<()> {
    let cfg = Config::load_global()?;
    let path = Config::global_path().context("no mecha home to find config.toml in")?;
    let features: Vec<Feature> = if on {
        feature::plan_enable(&cfg, ids).map_err(anyhow::Error::msg)?
    } else {
        let mut out = Vec::new();
        for id in ids {
            let f = Feature::parse(id).with_context(|| {
                format!("`{id}` is not a feature — `mecha features` lists them")
            })?;
            anyhow::ensure!(
                f.has_switch(),
                "`{id}` is part of `{}` and has no switch of its own",
                f.part_of().map(|p| p.id()).unwrap_or_default()
            );
            out.push(f);
        }
        out
    };
    let changes: Vec<(Feature, bool)> = features.iter().map(|f| (*f, on)).collect();
    feature::write_switches(&path, &changes)?;
    let names: Vec<String> = features.iter().map(|f| format!("`{}`", f.id())).collect();
    println!(
        "{} {} in {}",
        if on { "enabled" } else { "disabled" },
        names.join(", "),
        path.display()
    );
    if !on {
        // What a disable leaves blocked, said rather than discovered.
        for dependent in Feature::ALL.iter().filter(|g| {
            g.has_switch()
                && feature::switch(&cfg, **g) == Some(Switch::On)
                && !features.contains(g)
                && g.requires().iter().any(|d| features.contains(d))
        }) {
            println!("  `{}` needs it, and now reads blocked", dependent.id());
        }
    }
    println!("Services that are already running pick this up when they restart.");
    Ok(())
}

/// What the global configuration says about `f` right now, if it refuses:
/// the same [`feature::refusal`] the web routes answer with. `None` when
/// `f` is on, not guarded yet, or the configuration cannot be read — only
/// for wording a message ([`graph_tool_absent`]); a guard uses [`require`],
/// which refuses on an unreadable file.
pub fn refusal_now(f: Feature) -> Option<feature::Refusal> {
    let home = mecha_core::work::mecha_home().ok()?;
    let cfg = Config::load_global().ok()?;
    feature::refusal(&feature::Facts::read(&home, &cfg), f)
}

/// The first thing a verb that belongs to `f` does (FEATURES-DESIGN.md §4.2
/// item 5): refuse with the one sentence every surface says — *"Mail and
/// calendar is off (not enabled in [features]) — `mecha features enable
/// mail`"* — read from the global file, never the layered `Config` the verb
/// then runs with, which a project could otherwise answer for the owner.
///
/// **An unreadable configuration refuses** — the guard cannot run, so the
/// verb does not either. `refusal_now`'s "the verb reports it a moment
/// later" did not hold for every caller: `imagelib` never loads a config,
/// so a malformed file let `imagelib remove` through with `image = false`
/// written in it (review of #451).
pub fn require(f: Feature) -> Result<()> {
    let home = mecha_core::work::mecha_home()?;
    let cfg = Config::load_global().with_context(|| {
        format!(
            "cannot tell whether `{}` is switched on — the global configuration did not load",
            f.switch_owner().id()
        )
    })?;
    match feature::refusal(&feature::Facts::read(&home, &cfg), f) {
        Some(r) => anyhow::bail!("{}", r.sentence()),
        None => Ok(()),
    }
}

/// Why a knowledge-graph tool is missing from a run's surface: the switch,
/// when that is the cause — `prepare_tools` does not connect the graph
/// server with `graph` off, and asking about `[[mcp]]` would send the owner
/// to the wrong file — else the missing server entry (the #445 leftover).
pub fn graph_tool_absent(f: Feature, tool: &str) -> String {
    match refusal_now(f) {
        Some(r) => r.sentence(),
        None => format!(
            "no knowledge-graph server in this configuration — `{tool}` is not on the \
             tool surface. Is `[[mcp]]` enabled?"
        ),
    }
}

/// The upgrade notice, printed on stderr when a session or a service
/// starts: one line per feature this install set up whose switch is still
/// unanswered, and one per `[features]` key this build does not know.
///
/// Best-effort by design — a config that does not load is the command's own
/// error to report, a moment later, and a notice must never be the reason a
/// start fails.
pub fn print_notices() {
    let (Ok(home), Ok(cfg)) = (mecha_core::work::mecha_home(), Config::load_global()) else {
        return;
    };
    let facts = feature::Facts::read(&home, &cfg);
    if let Some(line) = notice_line(&feature::announcements(&facts)) {
        eprintln!("mecha: {line}");
    }
    for key in feature::unknown_switches(&cfg) {
        eprintln!("mecha: `[features] {key}` is not a feature this build knows — a newer build's, or a typo; ignored");
    }
}

/// The whole upgrade notice as one line, with one command that answers all of
/// it: nine lines on every start is a notice the owner learns to skip.
fn notice_line(announced: &[feature::Announcement]) -> Option<String> {
    if announced.is_empty() {
        return None;
    }
    let name = |a: &feature::Announcement| match &a.caveat {
        Some(why) => format!("{} ({why})", a.id.id()),
        None => a.id.id().to_string(),
    };
    // Split by what an unanswered switch does *today*: `Feature::gated` says
    // which are already off, and the rest still work until their surfaces
    // are guarded — calling those off would be a readout saying off while
    // the thing is on (found on review of #445).
    let off: Vec<String> = announced
        .iter()
        .filter(|a| a.id.gated())
        .map(name)
        .collect();
    let working: Vec<String> = announced
        .iter()
        .filter(|a| !a.id.gated())
        .map(name)
        .collect();
    let mut ids: Vec<&str> = Vec::new();
    for a in announced {
        for id in a.fix.split_whitespace().skip(3) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let mut parts = Vec::new();
    if !off.is_empty() {
        parts.push(format!("off until switched on: {}", off.join(", ")));
    }
    if !working.is_empty() {
        parts.push(format!(
            "still working, but will be off once their surfaces follow the switch: {}",
            working.join(", ")
        ));
    }
    Some(format!(
        "set up here but not switched on in [features] — {} — `mecha features enable {}`, or `mecha setup` to answer each",
        parts.join("; "),
        ids.join(" ")
    ))
}

fn plan(id: &str, json: bool, verify: bool) -> Result<()> {
    let f = Feature::parse(id)
        .with_context(|| format!("`{id}` is not a feature — `mecha features` lists them"))?;
    let p = sidecar::plan(
        f,
        &sidecar::Machinery::real()?,
        &recommend::Machine::read()?,
        &mecha_core::fetch::hub_dir()?,
        verify,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&p)?);
    } else {
        print!("{}", render_plan(&p));
    }
    Ok(())
}

fn bytes_text(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} GiB", b as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0} MiB", b as f64 / (1u64 << 20) as f64)
    }
}

/// `mecha features plan`: each sidecar and each pinned file, what this
/// machine already has, and what an install would fetch.
fn render_plan(p: &sidecar::Plan) -> String {
    use sidecar::{FileState, SidecarState};
    let mut out = format!("What `{}` runs beside mecha:\n", p.feature.id());
    for s in &p.sidecars {
        let state = match &s.state {
            SidecarState::Provided { by } => format!("provided — {by}; left alone"),
            SidecarState::Installed => "installed by mecha".to_string(),
            SidecarState::Incomplete => {
                "an install mecha began and did not finish — resumable".to_string()
            }
            SidecarState::Missing { step } => {
                format!("not here — its installer arrives in step {step}")
            }
            SidecarState::Unknown { why } => format!("unknown — {why}; nothing is offered over it"),
        };
        out.push_str(&format!("  {:<32} {state}\n", s.label));
    }
    out.push_str("\nThe models it loads:\n");
    for f in &p.files {
        let what = match (f.repo, f.path) {
            (Some(repo), path) => format!("{repo}/{path}"),
            (None, "") if f.model.is_empty() => "no recommended model".to_string(),
            (None, "") => f.model.to_string(),
            (None, path) => path.to_string(),
        };
        let state = match &f.state {
            FileState::Cached => "in the cache, matching its pin".to_string(),
            FileState::Download { bytes } => format!("to download, {}", bytes_text(*bytes)),
            FileState::Mismatch => {
                "a different file is at its path — not replaced, and not used".to_string()
            }
            FileState::WithSidecar => "comes with its sidecar's install".to_string(),
            FileState::HeldBy { sidecar } => format!("kept by the provided {sidecar}; not read"),
            FileState::Unverified => {
                "a file of the pinned size, placed by hand — not hashed by the plan".to_string()
            }
            FileState::NoRow => "no model is recommended for this machine's tier".to_string(),
        };
        out.push_str(&format!(
            "  {:<16} {what}\n  {:<16}   {state}\n",
            f.slot, ""
        ));
    }
    out.push('\n');
    if p.nothing_to_do {
        out.push_str("Nothing to install: this machine has all of it.\n");
        return out;
    }
    let unverified = p
        .files
        .iter()
        .filter(|f| matches!(f.state, FileState::Unverified))
        .count();
    if unverified > 0 {
        out.push_str(&format!(
            "{unverified} model file(s) placed by hand are not hashed — `mecha features plan {} --verify` reads them.\n",
            p.feature.id()
        ));
    }
    let mismatched = p
        .files
        .iter()
        .filter(|f| matches!(f.state, FileState::Mismatch))
        .count();
    if mismatched > 0 {
        out.push_str(&format!(
            "{mismatched} model file(s) at their path do not match their pins — move them aside for the pinned ones.\n"
        ));
    }
    if p.files.iter().any(|f| matches!(f.state, FileState::NoRow)) {
        out.push_str("No model is recommended at this machine's tier — see the hardware page.\n");
    }
    if p.download_bytes > 0 {
        out.push_str(&format!("To download: {}.\n", bytes_text(p.download_bytes)));
    }
    if p.sidecars
        .iter()
        .any(|s| matches!(s.state, SidecarState::Missing { .. }))
    {
        out.push_str("Installing is not built yet — each missing line names its step.\n");
    }
    out
}

fn gib(mb: u64) -> String {
    format!("{:.1} GiB", mb as f64 / 1024.0)
}

fn peak_text(p: &Peak) -> String {
    match p {
        Peak::Measured { mb, machine, date } => {
            format!("{}  measured on {machine}, {date}", gib(*mb as u64))
        }
        Peak::Arithmetic { mb } => format!("{}  arithmetic", gib(*mb as u64)),
        Peak::Unmeasured => "unmeasured".to_string(),
    }
}

fn sum_text(name: &str, s: &Sum) -> String {
    let part = |v: Option<u64>, floor: &Floor| match v {
        Some(mb) => format!(
            "{} of {} ({:.0}%)",
            gib(mb),
            gib(s.total_mb),
            mb as f64 * 100.0 / s.total_mb.max(1) as f64
        ),
        None => format!(
            "unknown of {} — unmeasured: {}; the rest add up to {}",
            gib(s.total_mb),
            floor.unknown.join(", "),
            gib(floor.known_mb)
        ),
    };
    let band = s
        .band
        .map(|b| format!(" — {}", b.word()))
        .unwrap_or_default();
    format!(
        "{name}\n  resident:          {}\n  everything loaded: {}{band}\n",
        part(s.resident_mb, &s.resident_floor),
        part(s.loaded_mb, &s.loaded_floor)
    )
}

/// `mecha features --probe`: every model the shown features need, what each
/// holds and on what evidence, and the sums against this machine.
fn render_budget(b: &Budget) -> String {
    let mut out = String::new();
    let tier = match b.tier_gb {
        Some(t) => format!("the {t} GB tier"),
        None => "below the smallest tier (16 GB)".to_string(),
    };
    match b.machine {
        Machine::Unified {
            total_mb,
            gpu_unread,
        } => {
            out.push_str(&format!("One memory pool, {} — {tier}.\n", gib(total_mb)));
            if gpu_unread {
                out.push_str(
                    "  (Only `nvidia-smi` is asked about a card, and it did not answer; a card \
                     it cannot see is counted as part of this pool.)\n",
                );
            }
        }
        Machine::Discrete {
            gpu_mb,
            cards,
            host_mb,
        } => out.push_str(&match cards {
            1 => format!(
                "A separate GPU: {} on the card, {} of system memory — {tier}.\n",
                gib(gpu_mb),
                gib(host_mb)
            ),
            n => format!(
                "{n} GPUs, the largest {} — read by that one card, since a model that must sit \
                 on one device cannot split — and {} of system memory — {tier}.\n",
                gib(gpu_mb),
                gib(host_mb)
            ),
        }),
    }
    out.push('\n');
    for l in &b.lines {
        let model = l.model.unwrap_or("no model recommended at this tier");
        out.push_str(&format!(
            "  {:<16} {model} — {}\n",
            l.label,
            l.residency.word().to_lowercase()
        ));
        match &l.host {
            None => out.push_str(&format!("  {:<16}   {}\n", "", peak_text(&l.gpu))),
            Some(h) => {
                out.push_str(&format!("  {:<16}   card: {}\n", "", peak_text(&l.gpu)));
                out.push_str(&format!("  {:<16}   host: {}\n", "", peak_text(h)));
            }
        }
        if !l.own_row && l.model.is_some() {
            out.push_str(&format!(
                "  {:<16}   (no row for this machine: the nearest row's figure, carried as arithmetic)\n",
                ""
            ));
        }
    }
    out.push('\n');
    match &b.host {
        None => out.push_str(&sum_text("The pool", &b.gpu)),
        Some(h) => {
            out.push_str(&sum_text("The card", &b.gpu));
            out.push_str(&sum_text("System memory", h));
        }
    }
    if !b.not_counted.is_empty() {
        out.push_str("\nNot in these sums:\n");
        for n in &b.not_counted {
            out.push_str(&format!("  {n}\n"));
        }
    }
    out
}

/// Depth under the top level: a part of a part (`layout` under `ocr`)
/// indents twice.
fn depth(f: Feature) -> usize {
    std::iter::successors(f.part_of(), |p| p.part_of()).count()
}

fn render(rows: &[Row], announced: &[feature::Announcement]) -> String {
    let width = rows
        .iter()
        .map(|r| 2 * depth(r.id) + r.id.id().len())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for row in rows {
        let name = format!("{}{}", "  ".repeat(depth(row.id)), row.id.id());
        let announcement = announced.iter().find(|a| a.id == row.id);
        let (say, fix) = match &row.state {
            // Set up here and unanswered: its own row, not a bare `off`.
            State::Off { .. } if announcement.is_some() => {
                let a = announcement.expect("checked");
                (
                    match &a.caveat {
                        Some(why) => format!("not enabled, but set up here ({why})"),
                        None => "not enabled, but set up here".to_string(),
                    },
                    Some(a.fix.clone()),
                )
            }
            State::On { detail } => (detail.clone(), None),
            State::Off { reason, fix } | State::Unready { reason, fix } => {
                (reason.clone(), fix.clone())
            }
            // The command of what blocks it, the same `next` the web app shows.
            State::Blocked { on } => (format!("needs {}", on.id()), row.next.clone()),
            State::Unknown { reason } => (reason.clone(), None),
        };
        out.push_str(&format!("{name:<width$}  {:<7}  {say}\n", row.state.word()));
        if let Some(fix) = fix {
            out.push_str(&format!("{:<width$}  {:<7}  → {fix}\n", "", ""));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The #445 leftover: a graph tool missing from a run's surface names the
    /// switch when that is why, and the `[[mcp]]` entry only when it is not —
    /// read from the global file, as every refusal is.
    #[test]
    fn a_missing_graph_tool_names_the_switch_when_that_is_the_cause() {
        let home = crate::testenv::HomeGuard::new("features-graph-absent");
        let write = |body: &str| std::fs::write(home.dir.join("config.toml"), body).unwrap();
        write("[features]\ngraph = false\n");
        let said = graph_tool_absent(Feature::Tasks, "kg_task_list");
        assert!(said.contains("`mecha features enable graph`"), "{said}");
        assert!(!said.contains("[[mcp]]"), "{said}");
        write("[features]\ngraph = true\n");
        let said = graph_tool_absent(Feature::Tasks, "kg_task_list");
        assert!(said.contains("Is `[[mcp]]` enabled?"), "{said}");
        assert!(require(Feature::Tasks).is_ok());
    }

    #[test]
    fn the_notice_is_one_line_with_one_command() {
        use mecha_core::feature::Announcement;
        assert_eq!(notice_line(&[]), None);
        let line = notice_line(&[
            Announcement {
                id: Feature::Mail,
                caveat: Some("no account is authorised".into()),
                fix: "mecha features enable mail".into(),
            },
            Announcement {
                id: Feature::Incognito,
                caveat: None,
                fix: "mecha features enable web incognito".into(),
            },
        ])
        .unwrap();
        // Every feature is guarded since step 3b, so both are called off.
        assert!(
            line.contains("off until switched on: mail (no account is authorised), incognito"),
            "{line}"
        );
        assert!(!line.contains("still working"), "{line}");
        assert!(
            line.contains("`mecha features enable mail web incognito`"),
            "{line}"
        );
    }

    #[test]
    fn parts_indent_under_their_parent_and_a_fix_gets_its_own_line() {
        // Every switch on, so each row shows its own settings' fix.
        let mut config = mecha_core::config::Config::default();
        for f in Feature::ALL.iter().filter(|f| f.has_switch()) {
            config.features.0.insert(f.id().to_string(), true);
        }
        let rows = feature::all(&feature::Facts {
            config,
            ..feature::Facts::default()
        });
        let text = render(&rows, &[]);
        assert!(text.contains("\n  tasks "), "{text}");
        assert!(text.contains("\n    layout "), "{text}");
        // Switched on without settings is unready, parts included, and never
        // a block on what depends on it (review of #449, pass 3).
        assert!(text.contains("unready"), "{text}");
        assert!(!text.contains("blocked"), "{text}");
        assert!(text.contains("→ add a [documents] table"), "{text}");
        assert!(text.contains("→ add an [image] table"), "{text}");
    }

    /// A card: two sums, the host one unknown because the chat row's host
    /// figure is unmeasured — said in words, never printed as a zero — and a
    /// model with no row of its own says its figure is carried.
    #[test]
    fn the_probe_on_a_card_shows_two_sums_and_says_what_it_does_not_know() {
        let b = recommend::budget(
            Machine::Discrete {
                gpu_mb: 32_768,
                cards: 1,
                host_mb: 65_536,
            },
            &[Feature::Ocr],
        );
        let text = render_budget(&b);
        assert!(
            text.contains(
                "A separate GPU: 32.0 GiB on the card, 64.0 GiB of system memory — the 32 GB tier."
            ),
            "{text}"
        );
        assert!(text.contains("The card\n"), "{text}");
        assert!(text.contains("System memory\n"), "{text}");
        // Each line's floor is its own: the on-demand OCR server's process
        // memory is on the everything-loaded line, never the resident one.
        assert!(
            text.contains("  resident:          unknown of 64.0 GiB — unmeasured: chat; the rest add up to 0.0 GiB\n"),
            "{text}"
        );
        assert!(
            text.contains("  everything loaded: unknown of 64.0 GiB — unmeasured: chat; the rest add up to 0.9 GiB\n"),
            "{text}"
        );
        assert!(text.contains("host: unmeasured"), "{text}");
        assert!(text.contains("the nearest row's figure, carried"), "{text}");
        assert!(!text.contains(" 0.0 GiB of 64"), "{text}");
    }

    /// One pool: one sum, with its band, and the exclusions named.
    #[test]
    fn the_probe_on_one_pool_names_what_it_leaves_out() {
        let b = recommend::budget(
            Machine::Unified {
                total_mb: 124_610,
                gpu_unread: false,
            },
            &[],
        );
        let text = render_budget(&b);
        assert!(
            text.contains("One memory pool, 121.7 GiB — the 128 GB tier."),
            "{text}"
        );
        assert!(text.contains("The pool\n"), "{text}");
        assert!(text.contains("— comfortable"), "{text}");
        assert!(text.contains("Not in these sums:\n  chat: "), "{text}");
        assert!(!text.contains("System memory"), "{text}");
    }

    /// Two cards are named as two, and read by the largest — never shown as
    /// one card holding their sum.
    #[test]
    fn the_probe_names_two_cards_and_reads_the_largest() {
        let b = recommend::budget(
            Machine::Discrete {
                gpu_mb: 24_576,
                cards: 2,
                host_mb: 65_536,
            },
            &[],
        );
        let text = render_budget(&b);
        assert!(text.starts_with("2 GPUs, the largest 24.0 GiB"), "{text}");
        assert!(text.contains("the 16 GB tier"), "{text}");
        assert!(!text.contains("48.0 GiB"), "{text}");
    }
}
