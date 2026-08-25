//! The `/entity` modal — repairing who is who in the knowledge graph.
//!
//! Tenth modal on the `/outbox` pattern, and it inherits that pattern's
//! shape: **read for display, and every mutation is a `mecha-graph …` child
//! process.** Nothing here reimplements a verb, so a thing the modal can do
//! is a thing a script can do.
//!
//! The gap it closes was found from the other side. Asked in the TUI to fix
//! a daughter's name, the model correctly reported that it could add an
//! alias and stage a fact correction and nothing else — no rename, no way to
//! create a person who has forty facts and no node. That was true of the
//! whole system, not just the tool surface: `merge_nodes` keeps the
//! survivor's name, so the workaround for a bad name needed a node that
//! nothing could create, and the two missing verbs were each other's only
//! workaround.
//!
//! Three decisions:
//!
//! - **The model still cannot do any of this, and that is the point.** The
//!   verbs went onto `mecha-graph` and this modal drives them as a person
//!   drives them — the same reasoning that keeps `kg_accept` off the MCP
//!   surface. A model that reads mail, web pages and Slack must not be able
//!   to rewrite who anyone in the graph *is*: an identity edit is invisible
//!   in a way a fact edit is not, because every fact about the node keeps
//!   reading correctly while pointing somewhere else.
//! - **Synchronous shell-outs.** An entity lookup against the graph measures
//!   7ms, so the `Watch`/detached-job machinery `/docs` needs for OAuth and
//!   `/outbox` needs for MCP startup would be ceremony around something
//!   faster than a keypress. If that ever stops being true this becomes a
//!   `Watch` like its siblings.
//! - **A refusal keeps the page open.** Every collision this can hit is a
//!   *question* — merge these two? did you mean the other node? — and
//!   answering it needs the page you were already reading. So a refusal
//!   lands in the status line and changes nothing else.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::{centered, list_height_reserving};

/// One resolved node, flattened for display.
pub struct EntityRow {
    pub id: String,
    pub name: String,
    pub node_type: String,
    pub aliases: Vec<String>,
    pub interactions: Option<i64>,
    pub facts: Vec<String>,
}

/// Which single-line edit is in flight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EditKind {
    Rename,
    Alias,
    NewPerson,
}

impl EditKind {
    pub fn title(self) -> &'static str {
        match self {
            EditKind::Rename => " rename to — enter confirms · esc cancels ",
            EditKind::Alias => " add alias — enter confirms · esc cancels ",
            EditKind::NewPerson => " new person, their name — enter confirms · esc cancels ",
        }
    }

    /// The `mecha-graph` verb behind it. One place, so the modal and the
    /// command line cannot drift into meaning different things by the same
    /// key.
    pub fn verb(self) -> &'static str {
        match self {
            EditKind::Rename => "rename",
            EditKind::Alias => "alias",
            EditKind::NewPerson => "new-person",
        }
    }
}

pub struct EntityModal {
    /// A row marked as the survivor of a merge. Two keystrokes on two rows
    /// rather than a form: the thing being merged is *these two nodes on
    /// screen*, and a text field asking for an id would be answered by
    /// copying one off the display.
    pub merge_keep: Option<String>,
    /// A merge awaiting y/n: (keep id, keep name, dup id, dup name).
    /// Merging is the one irreversible verb here, so it is the one that
    /// confirms — every other key on this modal is undoable by another key.
    pub merge_confirm: Option<(String, String, String, String)>,
    /// The lookup box.
    pub query: String,
    /// What the last lookup returned.
    pub rows: Vec<EntityRow>,
    pub selected: usize,
    /// An edit in flight: its kind and the text typed so far.
    pub edit: Option<(EditKind, String)>,
    pub status: Option<String>,
    /// True before the first lookup, so an empty list can say "type a name"
    /// rather than "no matches" — the two are opposite findings and the
    /// store-reader rule applies to a list as much as to a queue depth.
    pub fresh: bool,
    pub help: bool,
}

impl EntityModal {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
            edit: None,
            merge_keep: None,
            merge_confirm: None,
            status: None,
            fresh: true,
            help: false,
        }
    }

    pub fn selected_row(&self) -> Option<&EntityRow> {
        self.rows.get(self.selected)
    }

    pub fn move_sel(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        let next = (self.selected as isize + delta).rem_euclid(n);
        self.selected = next as usize;
    }

    /// Fold a `mecha-graph entity … --json` answer into the list.
    pub fn install(&mut self, json: &str) {
        self.rows = parse_rows(json);
        self.selected = 0;
        self.fresh = false;
        self.status = Some(match self.rows.len() {
            0 => format!(
                "nothing matches {:?} — ctrl-n creates a person by that name",
                self.query
            ),
            1 => "1 match".to_string(),
            n => format!("{n} matches"),
        });
    }

    pub fn draw(&self, frame: &mut Frame) {
        if self.help {
            self.draw_help(frame);
            return;
        }
        let area = frame.area();
        // Three reserved rows: query, keys and status. The
        // `list_height` rule — an inline clamp here saturates to zero on a
        // four-row terminal and panics on `min > max`.
        let rows = list_height_reserving(self.body_lines() as u16, area.height, 3);
        let box_area = centered(area, area.width.saturating_sub(6).min(110), rows);
        frame.render_widget(Clear, box_area);

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(vec![
            Span::styled("  search  ", Style::new().fg(Color::DarkGray)),
            Span::styled(
                if self.query.is_empty() {
                    "…".to_string()
                } else {
                    self.query.clone()
                },
                Style::new().fg(Color::White).bold(),
            ),
        ]));

        if self.rows.is_empty() {
            lines.push(Line::styled(
                if self.fresh {
                    "  type a name and press enter"
                } else {
                    "  no match"
                },
                Style::new().fg(Color::DarkGray),
            ));
        }
        for (i, row) in self.rows.iter().enumerate() {
            let here = i == self.selected;
            let keeping = self.merge_keep.as_deref() == Some(row.id.as_str());
            let marker = if keeping {
                "◆ "
            } else if here {
                "▸ "
            } else {
                "  "
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{marker}{:<8} ", row.node_type),
                    Style::new().fg(if here { Color::Cyan } else { Color::DarkGray }),
                ),
                Span::styled(
                    format!("{:<34} ", clip(&row.name, 34)),
                    if here {
                        Style::new().fg(Color::White).bold()
                    } else {
                        Style::new().fg(Color::White)
                    },
                ),
                Span::styled(
                    match row.interactions {
                        Some(n) => format!("{n} interactions"),
                        None => String::new(),
                    },
                    Style::new().fg(Color::DarkGray),
                ),
            ]));
            if here {
                if !row.aliases.is_empty() {
                    lines.push(Line::styled(
                        format!("      aka {}", clip(&row.aliases.join(", "), 88)),
                        Style::new().fg(Color::DarkGray),
                    ));
                }
                // A couple of facts, so the person deciding whether this is
                // the right node can see what is filed under it. Renaming
                // the wrong node is the failure this whole surface exists to
                // avoid, and the id alone does not prevent it.
                for f in row.facts.iter().take(3) {
                    lines.push(Line::styled(
                        format!("      · {}", clip(f, 88)),
                        Style::new().fg(Color::DarkGray),
                    ));
                }
            }
        }

        // The keys live INSIDE the box, not in the border title. A border
        // title is truncated to the box width without saying so, and the
        // keys that get cut are the ones at the end — which is how `m
        // merge` came to be advertised nowhere at all while the feature
        // shipped. A line in the body wraps instead of vanishing.
        lines.push(Line::styled(
            match (&self.edit, &self.merge_confirm, &self.merge_keep) {
                (Some((kind, buf)), ..) => format!("  {}  {buf}▌", kind.title()),
                (_, Some((_, keep, _, dup)), _) => format!(
                    "  merge {dup:?} INTO {keep:?}?  THIS CANNOT BE UNDONE  ·  y confirm  ·  any other key cancels"
                ),
                (_, _, Some(_)) => {
                    "  ◆ keeping this one — move to the duplicate and press m again  ·  esc cancels"
                        .to_string()
                }
                // "new person" read as "new search" to the person who hit
                // it and got a node named after their query. CREATE is the
                // verb that cannot be misread, and esc now has a first
                // meaning worth advertising.
                (..) if self.rows.is_empty() && self.query.is_empty() => {
                    "  type a name · enter search · ctrl-n CREATE a person · ? help · esc close"
                        .to_string()
                }
                _ => "  ↑↓ · enter search · r rename · a alias · m merge · ctrl-n CREATE person · esc clear · ? help"
                    .to_string(),
            },
            Style::new().fg(if self.merge_confirm.is_some() {
                Color::Red
            } else if self.edit.is_some() || self.merge_keep.is_some() {
                Color::Yellow
            } else {
                Color::DarkGray
            }),
        ));

        lines.push(Line::styled(
            match &self.status {
                Some(s) => format!("  {s}"),
                None => String::new(),
            },
            Style::new().fg(Color::Yellow),
        ));

        let title = " /entity — who is who in the knowledge graph ".to_string();

        let border = if self.merge_confirm.is_some() {
            Color::Red
        } else if self.edit.is_some() || self.merge_keep.is_some() {
            Color::Yellow
        } else {
            Color::Cyan
        };
        frame.render_widget(
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(border))
                        .title(title),
                )
                .wrap(Wrap { trim: false }),
            box_area,
        );
    }

    fn body_lines(&self) -> usize {
        // +1 for the key line, which is part of the body now rather than
        // the border.
        let mut n = 2 + self.rows.len().max(1);
        if let Some(row) = self.selected_row() {
            if !row.aliases.is_empty() {
                n += 1;
            }
            n += row.facts.len().min(3);
        }
        n
    }

    fn draw_help(&self, frame: &mut Frame) {
        let area = frame.area();
        let text = "\
  /entity — who is who in the knowledge graph

  enter     look the typed name up
  ↑ ↓       move through the matches
  m         merge: press once to mark the node to KEEP, then move to the
            duplicate and press m again. Confirms with y/n — it is the only
            irreversible action here
  r         rename the selected node
            the old name is kept as an alias, so everything that
            reached it by the old name still does
  a         add an alias to the selected node
  ctrl-n    CREATE a person, prefilled with what you typed — for someone
            who has facts and episodes but no node of their own. This
            writes a new entity; it does not start a new search
  esc       clear the search; again to close the modal
  ?         this
  esc       back

  Every one of these runs `mecha-graph` as a child process, so
  anything here is available to a script. The model cannot do any
  of it: an identity edit is invisible in a way a fact edit is not,
  because every fact about a node keeps reading correctly while
  pointing somewhere else.

  A name that is already another node's is refused rather than
  guessed at — that is a merge question, and `mecha-graph merge`
  is the verb that answers it.";
        let rows = list_height_reserving(text.lines().count() as u16, area.height, 0);
        let box_area = centered(area, area.width.saturating_sub(6).min(78), rows);
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(" /entity — keys · esc back "),
                )
                .wrap(Wrap { trim: false }),
            box_area,
        );
    }
}

/// Parse `mecha-graph entity … --json`.
///
/// A shape that cannot be read is an empty list *plus* a status the caller
/// sets — never a panic, and never a silent success. This is display code
/// for a store another program owns, and the ordinary failure is a version
/// skew rather than corruption.
fn parse_rows(json: &str) -> Vec<EntityRow> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return Vec::new();
    };
    items
        .iter()
        .map(|it| EntityRow {
            id: it["id"].as_str().unwrap_or_default().to_string(),
            name: it["name"].as_str().unwrap_or_default().to_string(),
            node_type: it["node_type"].as_str().unwrap_or("?").to_string(),
            aliases: it["aliases"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            interactions: it["interactions"].as_i64(),
            facts: it["facts"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|f| f["statement"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> &'static str {
        r#"[{"id":"person-1","name":"Josephine B. Conley","node_type":"person",
            "aliases":["josephine","josephine chang"],"interactions":1035,
            "facts":[{"statement":"Josephine is one of Luke's twin daughters."}]}]"#
    }

    #[test]
    fn a_lookup_answer_becomes_rows() {
        let mut m = EntityModal::new();
        m.query = "Josephine".into();
        m.install(sample());
        assert_eq!(m.rows.len(), 1);
        assert_eq!(m.rows[0].id, "person-1");
        assert_eq!(m.rows[0].interactions, Some(1035));
        assert_eq!(m.rows[0].facts.len(), 1);
        assert!(!m.fresh);
    }

    /// "Nothing matches" and "you have not searched yet" are opposite
    /// findings, and a list that rendered them alike would be the
    /// unreadable-store bug one layer up.
    #[test]
    fn an_empty_list_before_and_after_a_search_read_differently() {
        let m = EntityModal::new();
        assert!(m.fresh, "a modal that has not searched is not a no-match");
        let mut m = m;
        m.query = "Nobody".into();
        m.install("[]");
        assert!(!m.fresh);
        assert!(m.status.as_ref().unwrap().contains("nothing matches"));
    }

    /// Display code for another program's store degrades rather than
    /// panicking: version skew is the ordinary failure here.
    #[test]
    fn unreadable_json_is_an_empty_list_not_a_panic() {
        assert!(parse_rows("not json").is_empty());
        assert!(parse_rows("{}").is_empty());
        assert!(parse_rows("[]").is_empty());
        // A row missing every optional field still parses.
        let rows = parse_rows(r#"[{"id":"x"}]"#);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].node_type, "?");
        assert!(rows[0].interactions.is_none());
    }

    #[test]
    fn selection_wraps_and_survives_an_empty_list() {
        let mut m = EntityModal::new();
        m.move_sel(1); // must not panic on an empty list
        assert_eq!(m.selected, 0);
        m.install(sample());
        m.move_sel(1);
        assert_eq!(m.selected, 0, "one row wraps to itself");
        m.move_sel(-1);
        assert_eq!(m.selected, 0);
    }

    /// Each key means exactly one `mecha-graph` verb, defined once.
    #[test]
    fn every_edit_names_its_verb() {
        assert_eq!(EditKind::Rename.verb(), "rename");
        assert_eq!(EditKind::Alias.verb(), "alias");
        assert_eq!(EditKind::NewPerson.verb(), "new-person");
    }

    /// Render the modal to a buffer and read the text back. The merge key
    /// shipped advertised NOWHERE — a patch to the border title silently
    /// failed to apply and nothing asserted on what a user actually sees.
    /// A test that reads the rendered surface is the only kind that catches
    /// that; one asserting on the format string would have passed.
    fn rendered(m: &EntityModal, w: u16, h: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(w, h);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| m.draw(f)).unwrap();
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    }

    #[test]
    fn every_key_the_modal_answers_to_is_visible_on_it() {
        let mut m = EntityModal::new();
        m.install(sample());
        let screen = rendered(&m, 130, 24);
        for key in [
            "r rename",
            "a alias",
            "m merge",
            "ctrl-n CREATE person",
            "esc clear",
            "? help",
        ] {
            assert!(
                screen.contains(key),
                "{key:?} is not shown anywhere:\n{screen}"
            );
        }
    }

    /// An empty modal advertises the two things reachable from it, and
    /// names ctrl-n as CREATE — the word that cannot be read as "search".
    /// It was "new person" when somebody pressed it looking for a way to
    /// search and got a node named after their query.
    #[test]
    fn an_empty_modal_says_what_ctrl_n_actually_does() {
        let m = EntityModal::new();
        let screen = rendered(&m, 130, 24);
        assert!(screen.contains("ctrl-n CREATE a person"), "{screen}");
        assert!(screen.contains("enter search"), "{screen}");
    }

    /// The two merge states say what to do next, and the irreversible one
    /// says so in as many words.
    #[test]
    fn the_merge_states_explain_themselves() {
        let mut m = EntityModal::new();
        m.install(sample());

        m.merge_keep = Some("person-1".into());
        let marked = rendered(&m, 130, 24);
        assert!(marked.contains("press m again"), "{marked}");

        m.merge_confirm = Some((
            "person-1".into(),
            "Grace Choi".into(),
            "person-2".into(),
            "Youn Ji Choi".into(),
        ));
        let confirming = rendered(&m, 130, 24);
        assert!(confirming.contains("CANNOT BE UNDONE"), "{confirming}");
        assert!(confirming.contains("y confirm"), "{confirming}");
        assert!(
            confirming.contains("Youn Ji Choi") && confirming.contains("Grace Choi"),
            "the confirmation must name both sides: {confirming}"
        );
    }

    /// The `list_height` rule: the assertion is the draw itself. A modal
    /// that panics on a shrunken terminal takes the session down, partial
    /// answer and all.
    #[test]
    fn it_draws_at_tiny_sizes() {
        let mut m = EntityModal::new();
        m.install(sample());
        for (w, h) in [(1, 1), (4, 2), (10, 4), (20, 5), (80, 24)] {
            let backend = ratatui::backend::TestBackend::new(w, h);
            let mut term = ratatui::Terminal::new(backend).unwrap();
            term.draw(|f| m.draw(f)).unwrap();
            m.help = true;
            term.draw(|f| m.draw(f)).unwrap();
            m.help = false;
        }
    }
}
