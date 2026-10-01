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

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
    /// Machine output: one object per feature, `state` naming which of
    /// on / off / blocked / unready / unknown it is.
    #[arg(long)]
    pub json: bool,
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
}

pub fn execute(args: Args) -> Result<()> {
    match args.cmd {
        Some(Cmd::Enable { ids }) => return set(&ids, true),
        Some(Cmd::Disable { ids }) => return set(&ids, false),
        None => {}
    }
    let home = mecha_core::work::mecha_home()?;
    // The global config only, as `setup` and `doctor` read it: which
    // features an install has is a property of the machine, and `[features]`
    // is stripped from project layers anyway.
    let cfg = Config::load_global()?;
    let facts = feature::Facts::read(&home, &cfg);
    let rows = feature::all(&facts);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    for key in feature::unknown_switches(&cfg) {
        eprintln!("mecha: `[features] {key}` is not a feature this build knows — a newer build's, or a typo; ignored");
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
}
