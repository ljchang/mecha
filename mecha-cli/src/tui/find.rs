//! The /find modal: search the knowledge graph from the TUI.
//!
//! The `/tasks` pattern — every fetch drives `mecha kg … --json` as a child
//! process (on a thread, through a watch: a search starts an MCP server and
//! may take a second, and nothing here may freeze the event loop). The modal
//! renders what the child said and nothing else; there is no way for this
//! surface to reach the graph some way the command line cannot.
//!
//! The model already searches the graph through `kg_search`; this is the same
//! read with a person at the keyboard. Nothing rendered here reaches a model,
//! so an episode's words — which came out of mail or Slack — stay on the
//! human's side of the boundary, exactly as `/tasks` and `/queues` keep
//! theirs.
//!
//! Two states, search-tool shaped: typing edits the query and Enter runs it;
//! in the results, j/k move, Enter opens (an entity fetches its full record,
//! a fact or episode shows its text in place), and `/` goes back to the
//! query. Esc peels one layer, like every sibling.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// One hit: an entity the pack named, or a fact/episode item.
pub struct FindRow {
    /// `entity`, `fact`, or `episode` — the pack's own word.
    pub kind: String,
    /// Occurrence date, when the item carries one.
    pub when: String,
    /// One flattened line for the list.
    pub line: String,
    /// The full text, for the in-place detail (facts and episodes).
    pub full: String,
    /// Set when Enter should fetch the entity's record.
    pub entity: Option<String>,
}

pub struct FindModal {
    /// The query being typed or last run.
    pub query: String,
    /// Keys edit the query. Enter runs it and moves to the results.
    pub typing: bool,
    /// A search or entity fetch is in flight — shown, and Enter waits.
    pub loading: bool,
    pub rows: Vec<FindRow>,
    pub selected: usize,
    /// Rendered detail lines (an entity's record, or an item's full text).
    pub detail: Option<(String, Vec<String>)>,
    pub scroll: u16,
    pub status: Option<String>,
}

impl FindModal {
    pub fn new(query: Option<String>) -> Self {
        FindModal {
            typing: true,
            query: query.unwrap_or_default(),
            loading: false,
            rows: Vec::new(),
            selected: 0,
            detail: None,
            scroll: 0,
            status: None,
        }
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as i32 - 1;
        self.selected = (self.selected as i32 + delta).clamp(0, last) as usize;
    }

    pub fn selected_row(&self) -> Option<&FindRow> {
        self.rows.get(self.selected)
    }

    fn list_scroll(&self, visible: u16) -> u16 {
        let visible = visible.max(1) as usize;
        (self.selected + 1).saturating_sub(visible) as u16
    }

    pub fn draw(&self, frame: &mut Frame) {
        let width = 110u16.min(frame.area().width);
        // The detail is a document of its own, like the item detail one modal
        // over: full height, scrollable, Esc back to the results.
        if let Some((title, lines)) = &self.detail {
            let body: Vec<Line> = lines
                .iter()
                .map(|l| Line::styled(l.clone(), Style::new().fg(Color::White)))
                .collect();
            let height = super::list_height(body.len() as u16 + 1, frame.area().height);
            let area = super::centered(frame.area(), width, height);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(body)
                    .wrap(Wrap { trim: false })
                    .scroll((self.scroll, 0))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::new().fg(Color::Cyan))
                            .title(format!(" {title} — j/k scroll · Esc back ")),
                    ),
                area,
            );
            return;
        }

        let mut body: Vec<Line> = Vec::new();
        let cursor = if self.typing { "▎" } else { "" };
        body.push(Line::styled(
            format!("  search: {}{cursor}", self.query),
            Style::new().fg(if self.typing {
                Color::White
            } else {
                Color::DarkGray
            }),
        ));
        if let Some(s) = &self.status {
            body.push(Line::styled(
                format!("  {s}"),
                Style::new().fg(Color::Yellow),
            ));
        }
        if self.rows.is_empty() && !self.typing && !self.loading {
            body.push(Line::styled(
                "  nothing found — / edits the query",
                Style::new().fg(Color::DarkGray),
            ));
        }
        let header = body.len() as u16;
        for (i, r) in self.rows.iter().enumerate() {
            let sel = i == self.selected && !self.typing;
            let marker = if sel { "›" } else { " " };
            let text = format!(
                "{marker} {:<8} {:<10} {}",
                r.kind,
                r.when,
                truncate(&r.line, 84)
            );
            let style = if sel {
                Style::new().fg(Color::Black).bg(Color::Cyan)
            } else if r.kind == "entity" {
                Style::new().fg(Color::Cyan)
            } else {
                Style::new().fg(Color::White)
            };
            body.push(Line::styled(text, style));
        }
        let strip = if self.typing {
            "  type the query · Enter search · Esc close"
        } else {
            "  j/k move · Enter open · / edit query · Esc close"
        };
        body.push(Line::styled(
            strip.to_string(),
            Style::new().fg(Color::Cyan),
        ));

        let height = super::list_height(body.len() as u16, frame.area().height);
        let area = super::centered(frame.area(), width, height);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::Cyan))
            .title(" find — the knowledge graph ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 {
            return;
        }
        // Scroll keeps the selected row visible past the header lines.
        let scroll = self
            .list_scroll(inner.height.saturating_sub(header + 1))
            .min(body.len() as u16);
        frame.render_widget(Paragraph::new(body).scroll((scroll, 0)), inner);
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Rows out of a `mecha kg search --json` context pack: the entities the
/// pack named first (they are what Enter can open), then the items.
pub fn rows_from_pack(text: &str) -> anyhow::Result<Vec<FindRow>> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    let mut rows = Vec::new();
    for e in v["entities"].as_array().into_iter().flatten() {
        if let Some(name) = e.as_str() {
            rows.push(FindRow {
                kind: "entity".into(),
                when: "—".into(),
                line: name.to_string(),
                full: name.to_string(),
                entity: Some(name.to_string()),
            });
        }
    }
    for it in v["items"].as_array().into_iter().flatten() {
        let full = it["text"].as_str().unwrap_or("").to_string();
        rows.push(FindRow {
            kind: it["kind"].as_str().unwrap_or("?").to_string(),
            when: it["occurred_at"]
                .as_str()
                .map(|d| d.chars().take(10).collect::<String>())
                .unwrap_or_else(|| "—".into()),
            line: full.split_whitespace().collect::<Vec<_>>().join(" "),
            full,
            entity: None,
        });
    }
    Ok(rows)
}

/// An entity record out of `mecha kg entity --json`, rendered to lines —
/// the same facts the CLI prints, shaped for a box.
pub fn entity_detail(text: &str) -> anyhow::Result<(String, Vec<String>)> {
    let e: serde_json::Value = serde_json::from_str(text)?;
    let mut lines = Vec::new();
    if e["found"].as_bool() != Some(true) {
        return Ok(("not found".into(), vec!["no such entity".into()]));
    }
    if let Some(m) = e["ambiguous"].as_array().filter(|m| !m.is_empty()) {
        for c in m {
            lines.push(format!(
                "~ {}  ({})  last seen {}",
                c["name"].as_str().unwrap_or("?"),
                c["type"].as_str().unwrap_or("?"),
                c["last_seen"].as_str().unwrap_or("—"),
            ));
        }
        return Ok(("ambiguous — search the exact name".into(), lines));
    }
    let node = &e["node"];
    let title = node["name"].as_str().unwrap_or("entity").to_string();
    lines.push(format!(
        "{}  ·  {}",
        node["type"].as_str().unwrap_or("?"),
        node["id"].as_str().unwrap_or(""),
    ));
    if let Some(aliases) = node["aliases"].as_array().filter(|a| !a.is_empty()) {
        let a: Vec<&str> = aliases.iter().filter_map(|x| x.as_str()).collect();
        lines.push(format!("aka: {}", a.join(" · ")));
    }
    let i = &e["interaction"];
    if let Some(last) = i["last_seen_at"].as_str() {
        lines.push(format!(
            "seen {} times, last {last} via {}",
            i["interaction_count"].as_i64().unwrap_or(0),
            i["last_channel"].as_str().unwrap_or("?"),
        ));
    }
    if let Some(facts) = e["facts"].as_array().filter(|f| !f.is_empty()) {
        lines.push(String::new());
        lines.push("facts:".into());
        for f in facts {
            lines.push(format!("  · {}", f["statement"].as_str().unwrap_or("?")));
        }
    }
    if let Some(eps) = e["episodes"].as_array().filter(|x| !x.is_empty()) {
        lines.push(String::new());
        lines.push("episodes:".into());
        for ep in eps {
            lines.push(format!(
                "  {}  {}",
                ep["occurred_at"]
                    .as_str()
                    .map(|d| d.chars().take(10).collect::<String>())
                    .unwrap_or_else(|| "—".into()),
                ep["preview"]
                    .as_str()
                    .unwrap_or("")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            ));
        }
    }
    Ok((title, lines))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;

    /// Entities lead and items follow, and a multi-line episode body becomes
    /// one row — a listing where one result is forty lines buries the rest.
    #[test]
    fn a_pack_becomes_rows_entities_first_and_one_line_each() {
        let rows = rows_from_pack(
            r#"{"entities":["Courtney Rogers"],
                "items":[{"kind":"episode","occurred_at":"2026-08-03 18:55:21",
                          "text":"line one\nline two\nline three"}]}"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "entity");
        assert_eq!(rows[0].entity.as_deref(), Some("Courtney Rogers"));
        assert_eq!(rows[1].line, "line one line two line three");
        assert!(rows[1].full.contains('\n'), "the detail keeps the shape");
    }

    /// The ambiguous answer renders as candidates, never as a blank record —
    /// the `? (?)` bug the CLI shipped with, pinned at the modal too.
    #[test]
    fn an_ambiguous_entity_lists_its_candidates() {
        let (title, lines) = entity_detail(
            r#"{"found":true,"ambiguous":[
                {"name":"Courtney Rogers","type":"person","last_seen":"2026-08-21"},
                {"name":"Courtney A. Jimenez","type":"person","last_seen":"2021-06-30"}]}"#,
        )
        .unwrap();
        assert!(title.contains("ambiguous"));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("Courtney Rogers"));
    }

    /// The box must draw at sizes where the naive clamp panics — the shared
    /// rule of every modal, the assertion IS the draw.
    #[test]
    fn it_draws_at_tiny_sizes() {
        let mut m = FindModal::new(Some("dartmouth".into()));
        m.status = Some("searching…".into());
        m.rows = rows_from_pack(
            r#"{"entities":["Dartmouth"],
                "items":[{"kind":"fact","occurred_at":null,"text":"a fact"}]}"#,
        )
        .unwrap();
        for h in 1..=8u16 {
            for w in [8u16, 40, 120] {
                let backend = ratatui::backend::TestBackend::new(w, h);
                let mut term = Terminal::new(backend).unwrap();
                term.draw(|f| m.draw(f)).unwrap();
            }
        }
        m.typing = false;
        m.detail = Some(("Dartmouth".into(), vec!["org · org-1".into(); 12]));
        for h in 1..=8u16 {
            let backend = ratatui::backend::TestBackend::new(40, h);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| m.draw(f)).unwrap();
        }
    }
}
