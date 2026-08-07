//! The mecha block mark, for the TUI.
//!
//! Eight rows of 9 cells, matching `logo.svg`: a bar with the M's notch cut
//! into it, a gap, two legs broken at the knee, the slot in the upper half, and
//! the pointed feet. Block Elements and quadrants only.
//!
//! Rows are multi-byte — index by `chars()`, never by byte offset.

pub const MARK: [&str; 8] = [
    "███▙▄▟███",
    "█████████",
    "▀▀▀▀▀▀▀▀▀",
    "██ ▄▄▄ ██",
    "██ ▀▀▀ ██",
    "▀▀     ▀▀",
    "██▖   ▗██",
    "██▄▖ ▗▄██",
];

pub const WIDTH: usize = 9;
pub const HEIGHT: usize = 8;

/// Rows whose centre three cells are the slot, and so carry the accent.
const SLOT_ROWS: [usize; 2] = [3, 4];
const SLOT: std::ops::Range<usize> = 3..6;

const STRUCTURE: (u8, u8, u8) = (93, 82, 148); // accent-700 #5d5294
const ACCENT: (u8, u8, u8) = (181, 171, 252); // accent-400 #b5abfc
const HAZARD: (u8, u8, u8) = (232, 162, 74); // hazard     #e8a24a
const RESET: &str = "\x1b[0m";

/// What the slot is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The loop is running — accent.
    Running,
    /// A send is held for approval — hazard.
    Held,
}

fn fg((r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

/// The mark alone, one `String` per row.
pub fn mark(slot: Slot) -> Vec<String> {
    let structure = fg(STRUCTURE);
    let highlight = fg(match slot {
        Slot::Running => ACCENT,
        Slot::Held => HAZARD,
    });

    MARK.iter()
        .enumerate()
        .map(|(row, cells)| {
            let cells: Vec<char> = cells.chars().collect();
            if !SLOT_ROWS.contains(&row) {
                return format!("{structure}{}{RESET}", cells.iter().collect::<String>());
            }
            let take = |r: std::ops::Range<usize>| cells[r].iter().collect::<String>();
            format!(
                "{structure}{}{highlight}{}{structure}{}{RESET}",
                take(0..SLOT.start),
                take(SLOT),
                take(SLOT.end..WIDTH),
            )
        })
        .collect()
}

/// The mark with up to three lines of detail set beside it, as the startup splash.
///
/// ```text
/// ███▙▄▟███
/// █████████
/// ▀▀▀▀▀▀▀▀▀
/// ██ ▄▄▄ ██  mecha 0.4.0
/// ██ ▀▀▀ ██  anthropic · claude-opus-5
/// ▀▀     ▀▀  7 tools · sandbox: bwrap
/// ██▖   ▗██
/// ██▄▖ ▗▄██
/// ```
pub fn splash(detail: &[&str], slot: Slot) -> String {
    // Detail starts on the row the slot starts on, so it reads level with the mark.
    const FIRST_DETAIL_ROW: usize = 3;

    mark(slot)
        .into_iter()
        .enumerate()
        .map(|(row, art)| match row.checked_sub(FIRST_DETAIL_ROW).and_then(|i| detail.get(i)) {
            Some(line) => format!("{art}  {line}"),
            None => art,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_nine_cells_wide() {
        for row in MARK {
            assert_eq!(row.chars().count(), WIDTH, "row {row:?} is not {WIDTH} cells");
        }
        assert_eq!(MARK.len(), HEIGHT);
    }

    #[test]
    fn detail_sits_beside_the_slot() {
        let out = splash(&["mecha 0.4.0"], Slot::Running);
        let line = out.lines().nth(3).unwrap();
        assert!(line.ends_with("  mecha 0.4.0"));
    }
}
