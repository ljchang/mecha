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
                && g.requires().iter().any(|d| features.contains(d))
        }) {
            println!("  `{}` needs it, and now reads blocked", dependent.id());
        }
    }
    println!("Services that are already running pick this up when they restart.");
    Ok(())
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
    let named: Vec<String> = announced
        .iter()
        .map(|a| match &a.caveat {
            Some(why) => format!("{} ({why})", a.id.id()),
            None => a.id.id().to_string(),
        })
        .collect();
    let mut ids: Vec<&str> = Vec::new();
    for a in announced {
        for id in a.fix.split_whitespace().skip(3) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    Some(format!(
        // True while nothing is gated on the switch (FEATURES-DESIGN §9, step
        // 1a): these still work. Step 1b, which gates them, says "so off".
        "set up here but not switched on in [features]: {} — they still work, and will be off once switches are enforced; `mecha features enable {}`, or `mecha setup` to answer each",
        named.join(", "),
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
            State::Blocked { on } => (format!("needs {}", on.id()), None),
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
        assert!(
            line.contains("mail (no account is authorised), incognito"),
            "{line}"
        );
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
        assert!(text.contains("needs graph"), "{text}");
        assert!(text.contains("→ add an [image] table"), "{text}");
    }
}
