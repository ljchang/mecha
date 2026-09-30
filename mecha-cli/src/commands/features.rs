//! `mecha features` — which optional parts of mecha are on, and the way to
//! turn on each of the rest (`docs/FEATURES-DESIGN.md`, step 0).
//!
//! The human half of [`mecha_core::feature`]. Read-only, and **no network**:
//! the core module reads the config and the disk, never a server, because
//! several servers here start the moment they are asked.

use anyhow::Result;
use mecha_core::feature::{self, Feature, Row, State};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Machine output: one object per feature, `state` naming which of
    /// on / off / blocked / unready / unknown it is.
    #[arg(long)]
    pub json: bool,
}

pub fn execute(args: Args) -> Result<()> {
    let home = mecha_core::work::mecha_home()?;
    // The global config only, as `setup` and `doctor` read it: which
    // features an install has is a property of the machine, and every table
    // that turns one on is stripped from project layers anyway.
    let cfg = mecha_core::config::Config::load_global()?;
    let rows = feature::all(&cfg, &feature::Facts::read(&home));
    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    print!("{}", render(&rows));
    Ok(())
}

/// Depth under the top level: a part of a part (`layout` under `ocr`)
/// indents twice.
fn depth(f: Feature) -> usize {
    std::iter::successors(f.part_of(), |p| p.part_of()).count()
}

fn render(rows: &[Row]) -> String {
    let width = rows
        .iter()
        .map(|r| 2 * depth(r.id) + r.id.id().len())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for row in rows {
        let name = format!("{}{}", "  ".repeat(depth(row.id)), row.id.id());
        let (say, fix) = match &row.state {
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
    fn parts_indent_under_their_parent_and_a_fix_gets_its_own_line() {
        let rows = feature::all(
            &mecha_core::config::Config::default(),
            &feature::Facts::default(),
        );
        let text = render(&rows);
        assert!(text.contains("\n  tasks "), "{text}");
        assert!(text.contains("\n    layout "), "{text}");
        assert!(text.contains("needs graph"), "{text}");
        assert!(text.contains("→ add an [image] table"), "{text}");
    }
}
