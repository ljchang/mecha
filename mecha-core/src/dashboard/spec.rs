//! `dashboard.json`: the spec the model writes.
//!
//! Parsing is two passes over one document. The first walks the raw JSON for
//! what no type can express — a string anywhere that is an address, a string
//! too long to be anything but a payload. The second is the typed shape, with
//! unknown fields refused, and then the rules that relate one field to
//! another. Both passes report everything they find.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{is_identifier, pointer, vegalite, Refusal, Refusals};

/// The only spec version this build reads.
pub const VERSION: u32 = 1;

/// A spec is a page of panels, not a database; anything this large is data
/// that belongs in a loader.
pub const MAX_BYTES: usize = 256 * 1024;
const MAX_PANELS: usize = 48;
const MAX_DATASETS: usize = 16;
const MAX_TABLE_COLUMNS: usize = 24;
const MAX_FILTERS: usize = 16;
const MAX_TITLE: usize = 120;
/// Markdown in a text panel.
const MAX_MARKDOWN: usize = 8 * 1024;
/// Any other string: a title, a field name, a Vega expression.
const MAX_STRING: usize = 2_000;

/// A checked spec. Deliberately not `Deserialize`: the only way to get one is
/// [`Spec::parse`], so no caller can reach for `serde_json::from_value` and
/// hold a spec that skipped the screens — the property is in the type.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Spec {
    pub version: u32,
    pub title: String,
    /// A theme the owner wrote. The spec names one; it never carries a colour.
    pub theme: String,
    pub datasets: Vec<String>,
    pub filters: Vec<Filter>,
    pub panels: Vec<Panel>,
}

/// The wire shape `parse` reads before any check has run.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    version: u32,
    title: String,
    #[serde(default = "default_theme")]
    theme: String,
    datasets: Vec<String>,
    #[serde(default)]
    filters: Vec<Filter>,
    panels: Vec<Panel>,
}

impl From<Wire> for Spec {
    fn from(w: Wire) -> Self {
        Spec {
            version: w.version,
            title: w.title,
            theme: w.theme,
            datasets: w.datasets,
            filters: w.filters,
            panels: w.panels,
        }
    }
}

fn default_theme() -> String {
    "default".into()
}

/// A control the renderer draws over one column; every panel bound to the
/// dataset follows it. Rendered by us, never by a chart's `bind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub id: String,
    pub label: String,
    pub dataset: String,
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Panel {
    Kpi {
        title: String,
        dataset: String,
        value: KpiValue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<u8>,
    },
    Chart {
        title: String,
        /// A Vega-Lite view, checked against the subset in `vegalite.rs`.
        vegalite: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<u8>,
    },
    Table {
        title: String,
        dataset: String,
        columns: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<u8>,
    },
    Text {
        markdown: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<u8>,
    },
}

impl Panel {
    fn span(&self) -> Option<u8> {
        match self {
            Panel::Kpi { span, .. }
            | Panel::Chart { span, .. }
            | Panel::Table { span, .. }
            | Panel::Text { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KpiValue {
    pub op: KpiOp,
    /// Required for every op but `count`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KpiOp {
    Sum,
    Mean,
    Min,
    Max,
    Count,
    /// The value in the dataset's last row — "the latest reading".
    Last,
}

impl Spec {
    /// Parse and check a `dashboard.json`. Every refusal found is returned,
    /// not just the first.
    pub fn parse(text: &str) -> Result<Spec, Refusals> {
        if text.len() > MAX_BYTES {
            return Err(Refusals(vec![Refusal::new(
                "",
                format!(
                    "the spec is {} bytes; the limit is {MAX_BYTES} — data belongs in a loader, not the spec",
                    text.len()
                ),
            )]));
        }
        let raw: Value = serde_json::from_str(text)
            .map_err(|e| Refusals(vec![Refusal::new("", format!("not JSON: {e}"))]))?;

        let mut out = Vec::new();
        screen_strings(&raw, "", &mut out);
        let spec = match serde_json::from_value::<Wire>(raw) {
            Ok(wire) => Some(Spec::from(wire)),
            Err(e) => {
                out.push(Refusal::new(
                    "",
                    format!("does not match the spec's shape: {e}"),
                ));
                None
            }
        };
        if let Some(spec) = &spec {
            spec.check(&mut out);
        }
        match spec {
            Some(spec) if out.is_empty() => Ok(spec),
            _ => Err(Refusals(out)),
        }
    }

    fn check(&self, out: &mut Vec<Refusal>) {
        if self.version != VERSION {
            out.push(Refusal::new(
                "/version",
                format!(
                    "this build reads spec version {VERSION}, not {}",
                    self.version
                ),
            ));
        }
        if self.title.trim().is_empty() || self.title.chars().count() > MAX_TITLE {
            out.push(Refusal::new(
                "/title",
                format!("a title is 1–{MAX_TITLE} characters"),
            ));
        }
        if !is_theme_name(&self.theme) {
            out.push(Refusal::new(
                "/theme",
                "a theme is a name the owner's theme files use: [a-z0-9-], at most 40 characters",
            ));
        }

        if self.datasets.len() > MAX_DATASETS {
            out.push(Refusal::new(
                "/datasets",
                format!("at most {MAX_DATASETS} datasets"),
            ));
        }
        for (i, name) in self.datasets.iter().enumerate() {
            let at = format!("/datasets/{i}");
            if !is_identifier(name) {
                out.push(Refusal::new(
                    at,
                    format!("dataset names match [a-z][a-z0-9_]* (got {name:?})"),
                ));
            } else if self.datasets[..i].contains(name) {
                out.push(Refusal::new(
                    at,
                    format!("dataset {name:?} is declared twice"),
                ));
            }
        }
        let declared = |name: &str| self.datasets.iter().any(|d| d == name);

        if self.filters.len() > MAX_FILTERS {
            out.push(Refusal::new(
                "/filters",
                format!("at most {MAX_FILTERS} filters"),
            ));
        }
        for (i, filter) in self.filters.iter().enumerate() {
            let at = format!("/filters/{i}");
            if !is_identifier(&filter.id) {
                out.push(Refusal::new(
                    pointer(&at, "id"),
                    "filter ids match [a-z][a-z0-9_]*",
                ));
            } else if self.filters[..i].iter().any(|f| f.id == filter.id) {
                out.push(Refusal::new(
                    pointer(&at, "id"),
                    format!("filter id {:?} is used twice", filter.id),
                ));
            }
            if filter.label.trim().is_empty() {
                out.push(Refusal::new(
                    pointer(&at, "label"),
                    "a filter needs a label",
                ));
            }
            if !is_identifier(&filter.field) {
                out.push(not_a_column(pointer(&at, "field")));
            }
            if !declared(&filter.dataset) {
                out.push(undeclared(pointer(&at, "dataset"), &filter.dataset));
            }
        }

        if self.panels.is_empty() || self.panels.len() > MAX_PANELS {
            out.push(Refusal::new(
                "/panels",
                format!("a dashboard has 1–{MAX_PANELS} panels"),
            ));
        }
        for (i, panel) in self.panels.iter().enumerate() {
            let at = format!("/panels/{i}");
            if let Some(span) = panel.span() {
                if !(1..=4).contains(&span) {
                    out.push(Refusal::new(
                        pointer(&at, "span"),
                        "span is 1–4 grid columns",
                    ));
                }
            }
            match panel {
                Panel::Kpi {
                    title,
                    dataset,
                    value,
                    ..
                } => {
                    check_title(&at, title, out);
                    if !declared(dataset) {
                        out.push(undeclared(pointer(&at, "dataset"), dataset));
                    }
                    match (&value.op, &value.field) {
                        (KpiOp::Count, _) => {}
                        (_, None) => out.push(Refusal::new(
                            pointer(&at, "value"),
                            "every op but `count` needs a `field`",
                        )),
                        (_, Some(field)) if !is_identifier(field) => {
                            out.push(not_a_column(format!("{at}/value/field")))
                        }
                        (_, Some(_)) => {}
                    }
                }
                Panel::Chart {
                    title, vegalite, ..
                } => {
                    check_title(&at, title, out);
                    vegalite::check(vegalite, &self.datasets, &pointer(&at, "vegalite"), out);
                }
                Panel::Table {
                    title,
                    dataset,
                    columns,
                    ..
                } => {
                    check_title(&at, title, out);
                    if !declared(dataset) {
                        out.push(undeclared(pointer(&at, "dataset"), dataset));
                    }
                    if columns.is_empty() || columns.len() > MAX_TABLE_COLUMNS {
                        out.push(Refusal::new(
                            pointer(&at, "columns"),
                            format!("a table shows 1–{MAX_TABLE_COLUMNS} columns"),
                        ));
                    }
                    for (j, col) in columns.iter().enumerate() {
                        if !is_identifier(col) {
                            out.push(not_a_column(format!("{at}/columns/{j}")));
                        }
                    }
                }
                Panel::Text { markdown, .. } => {
                    if markdown.trim().is_empty() {
                        out.push(Refusal::new(
                            pointer(&at, "markdown"),
                            "a text panel needs text",
                        ));
                    }
                    if has_markdown_link(markdown) {
                        out.push(Refusal::new(
                            pointer(&at, "markdown"),
                            "a dashboard's text has no links — inline, autolink or reference — \
                             and a relative link is still a destination; write the words only",
                        ));
                    }
                    if has_html_tag(markdown) {
                        out.push(Refusal::new(
                            pointer(&at, "markdown"),
                            "a dashboard's text has no HTML — a tag can carry a relative link or \
                             a handler past every other check; write markdown prose only",
                        ));
                    }
                }
            }
        }
    }
}

fn check_title(at: &str, title: &str, out: &mut Vec<Refusal>) {
    if title.trim().is_empty() || title.chars().count() > MAX_TITLE {
        out.push(Refusal::new(
            pointer(at, "title"),
            format!("a panel title is 1–{MAX_TITLE} characters"),
        ));
    }
}

/// A column name is checked alone as well as against its loader, so a spec
/// parsed before its loaders are known still names only plausible columns.
fn not_a_column(at: String) -> Refusal {
    Refusal::new(
        at,
        "a column name matches [a-z][a-z0-9_]*, as a loader declares it",
    )
}

fn undeclared(at: String, name: &str) -> Refusal {
    Refusal::new(
        at,
        format!("dataset {name:?} is not in the spec's `datasets`"),
    )
}

fn is_theme_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

/// The pass no type can make: every string in the document, wherever it sits.
///
/// An address is refused anywhere — a chart title, a markdown link, a Vega
/// expression — because "the spec names no destination" is only true if it
/// holds for every string, not for the fields someone thought to check. The
/// one exception is Vega-Lite's own `$schema` marker, which vega-embed reads
/// to pick a parser and never fetches.
fn screen_strings(v: &Value, at: &str, out: &mut Vec<Refusal>) {
    match v {
        Value::String(s) => {
            let limit = if is_text_panel_markdown(at) {
                MAX_MARKDOWN
            } else {
                MAX_STRING
            };
            if s.chars().count() > limit {
                out.push(Refusal::new(
                    at,
                    format!("strings here are at most {limit} characters"),
                ));
            }
            if is_chart_schema_marker(at) && is_vegalite_schema(s) {
                return;
            }
            if has_character_reference(s) {
                out.push(Refusal::new(
                    at,
                    "write the character itself, not a character reference (&#…; or &name;) — \
                     references can spell a scheme the address check cannot see",
                ));
            }
            if s.to_ascii_lowercase().contains("url(") {
                out.push(Refusal::new(
                    at,
                    "a spec names no destinations: `url(` is a CSS fetch, relative or not",
                ));
            }
            if is_address(s) {
                out.push(Refusal::new(
                    at,
                    "a spec names no destinations: this string is an address (a URL, a \
                     protocol-relative path, or a data:/javascript: URI). Charts bind to a \
                     dataset by name; links are not part of a dashboard",
                ));
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                screen_strings(item, &format!("{at}/{i}"), out);
            }
        }
        Value::Object(map) => {
            for (k, item) in map {
                let here = pointer(at, k);
                // Keys are strings too: a style object accepts arbitrary keys,
                // so an address can sit in one as easily as in a value.
                if is_address(k) {
                    out.push(Refusal::new(
                        &here,
                        "a spec names no destinations: this key is an address",
                    ));
                }
                screen_strings(item, &here, out);
            }
        }
        _ => {}
    }
}

/// `/panels/<n>/vegalite/$schema` exactly — the one place the marker is exempt.
fn is_chart_schema_marker(at: &str) -> bool {
    let mut parts = at.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(""), Some("panels"), Some(n), Some("vegalite"), Some("$schema"), None)
            if n.chars().all(|c| c.is_ascii_digit())
    )
}

/// `/panels/<n>/markdown` exactly — the one field with the larger budget.
fn is_text_panel_markdown(at: &str) -> bool {
    let mut parts = at.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(""), Some("panels"), Some(n), Some("markdown"), None)
            if n.chars().all(|c| c.is_ascii_digit())
    )
}

fn is_vegalite_schema(s: &str) -> bool {
    s.starts_with("https://vega.github.io/schema/vega-lite/") && s.ends_with(".json")
}

/// Does this string name somewhere? Deliberately broad: a false refusal
/// costs the model one retry, a missed address costs the property.
///
/// A scheme counts only where a URI could start — at the beginning, or after
/// a character that is not part of a word — and only when something follows
/// it directly: `data:image/png` and `(javascript:x` are addresses, "Raw
/// data: counts" and "Profile: x" are titles.
///
/// Control characters are dropped first: the URL parser removes tab and
/// newline before parsing, so `java\tscript:` is `javascript:`. Character
/// references are not decoded here — a spec may not contain one at all
/// ([`has_character_reference`]). A *relative* destination (`/outbox/x`) is
/// not an address by this test; it can only become a destination as a link,
/// and links are refused as syntax ([`has_markdown_link`]) and dropped by the
/// renderer (design §4.1).
pub(crate) fn is_address(s: &str) -> bool {
    let lower = normalise(s);
    let lower = lower.as_str();
    // Anywhere, not only at the start: `[x](//host/p)` is a protocol-relative
    // link one character in. Nothing a spec legitimately says contains `//`.
    // Backslashes too: for a special scheme the URL parser reads `\` as `/`,
    // so `/\host/p` and `\\host/p` are `//host/p`. Two adjacent characters
    // rather than folding every `\`, which would turn a regex's `\\d` into a
    // false refusal.
    if lower.contains("www.")
        || lower
            .as_bytes()
            .windows(2)
            .any(|w| w.iter().all(|c| matches!(c, b'/' | b'\\')))
    {
        return true;
    }
    [
        // The URL parser supplies the slashes a special scheme leaves out, so
        // `http:host` is `http://host/`.
        "http:",
        "https:",
        "data:",
        "javascript:",
        "vbscript:",
        "blob:",
        "file:",
        "mailto:",
    ]
    .iter()
    .any(|scheme| {
        lower.match_indices(scheme).any(|(i, _)| {
            let starts_word = lower[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_ascii_alphanumeric());
            let followed = lower[i + scheme.len()..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace());
            starts_word && followed
        })
    })
}

/// Lowercase, controls dropped.
fn normalise(s: &str) -> String {
    s.trim()
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// A character reference — `&#106;`, `&#x6a;`, `&amp;`. CommonMark decodes
/// them in a link destination, so any letter of a scheme can hide behind one;
/// rather than decode them and hope the decoding matches the renderer's, a
/// spec may not contain one at all. JSON carries every character directly.
fn has_character_reference(s: &str) -> bool {
    s.match_indices('&').any(|(i, _)| {
        let rest = &s[i + 1..];
        let body = rest
            .strip_prefix('#')
            .map(|r| r.strip_prefix(['x', 'X']).unwrap_or(r));
        match body {
            Some(digits) => digits.chars().next().is_some_and(|c| c.is_ascii_hexdigit()),
            None => {
                let name: usize = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .count();
                name > 0 && rest[name..].starts_with(';')
            }
        }
    })
}

/// The start of an HTML tag, comment or declaration as CommonMark reads one:
/// `<` followed by a letter, `/`, `!` or `?`. Raw HTML is the fourth way to
/// write a link, and the one no link-syntax test sees — `<a href="/x">`,
/// `<img src=x onerror=…>` — so a text panel may not contain any. A `<`
/// before a space or a digit ("a < b", "<5 ms") is prose and stays.
fn has_html_tag(s: &str) -> bool {
    s.match_indices('<').any(|(i, _)| {
        s[i + 1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
    })
}

/// Markdown link syntax: an inline `[text](dest)`, an angle autolink, or a
/// reference definition `[label]: dest`. A dashboard's text renders no links
/// (design §4.1), so a spec that writes one is refused rather than silently
/// stripped — and this also covers a *relative* destination, which no scheme
/// test can see.
fn has_markdown_link(s: &str) -> bool {
    s.contains("](")
        || s.contains("<http")
        || s.lines().any(|line| {
            let t = line.trim_start();
            t.starts_with('[')
                && t.find("]:")
                    .is_some_and(|i| !t[1..i].is_empty() && !t[1..i].contains(']'))
        })
}
