//! A loader: a registered source, a read-only query, a declared shape, and a
//! schedule — and the check every refresh's output must pass.
//!
//! The declared columns are what review approves (design §3.2, §6.3). A
//! refresh that returns anything else — another column, a value of another
//! type, more rows than the cap — is refused rather than published, because a
//! query whose output has changed shape is a different loader than the one a
//! human released. [`Loader::digest`] is how "the one a human released" is
//! named: a refresh pushes only under the digest that was reviewed.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{is_identifier, Refusal, Refusals};
use crate::cron::Schedule;

/// Rows one dataset may hold. A dashboard draws pictures, not tables of
/// record; a query that needs more is aggregating in the wrong place.
pub const MAX_ROWS: u32 = 100_000;
const MAX_COLUMNS: usize = 64;
const MAX_QUERY: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loader {
    /// The file's stem, which is the dataset's name. Never read from the file:
    /// a name that can disagree with its filename is a bug with no upside.
    #[serde(skip)]
    pub name: String,
    /// A source the owner registered in `sources.toml`. Never a connection
    /// string — the model names a source, it never holds a credential.
    pub source: String,
    pub schedule: Schedule,
    /// IANA zone for the schedule; `None` is UTC. Never an offset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub max_rows: u32,
    pub query: String,
    #[serde(rename = "column")]
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ColumnType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    String,
    Integer,
    Float,
    Boolean,
    /// `YYYY-MM-DD`.
    Date,
    /// Stored as RFC 3339 in UTC.
    Timestamp,
}

impl ColumnType {
    fn as_str(self) -> &'static str {
        match self {
            ColumnType::String => "string",
            ColumnType::Integer => "integer",
            ColumnType::Float => "float",
            ColumnType::Boolean => "boolean",
            ColumnType::Date => "date (YYYY-MM-DD)",
            ColumnType::Timestamp => "timestamp (RFC 3339, or YYYY-MM-DD HH:MM:SS in UTC)",
        }
    }
}

impl Loader {
    /// Parse and check one `loaders/<name>.toml`.
    pub fn parse(name: &str, text: &str) -> Result<Loader, Refusals> {
        let mut loader: Loader = toml::from_str(text).map_err(|e| {
            Refusals(vec![Refusal::new(
                "",
                format!("does not match a loader's shape: {e}"),
            )])
        })?;
        loader.name = name.to_string();
        let mut out = Vec::new();
        loader.check(&mut out);
        if out.is_empty() {
            Ok(loader)
        } else {
            Err(Refusals(out))
        }
    }

    fn check(&self, out: &mut Vec<Refusal>) {
        if !is_identifier(&self.name) {
            out.push(Refusal::new(
                "",
                format!(
                    "a loader's file name is its dataset's name and matches [a-z][a-z0-9_]* (got {:?})",
                    self.name
                ),
            ));
        }
        if !is_identifier(&self.source) {
            out.push(Refusal::new(
                "/source",
                "`source` names a source registered in sources.toml: [a-z][a-z0-9_]*",
            ));
        }
        if let Some(tz) = &self.timezone {
            if tz.parse::<Tz>().is_err() {
                out.push(Refusal::new(
                    "/timezone",
                    format!("{tz:?} is not an IANA zone name (e.g. America/New_York); offsets are refused"),
                ));
            }
        }
        if self.max_rows == 0 || self.max_rows > MAX_ROWS {
            out.push(Refusal::new(
                "/max_rows",
                format!("max_rows is 1–{MAX_ROWS}"),
            ));
        }
        if self.query.trim().is_empty() || self.query.len() > MAX_QUERY {
            out.push(Refusal::new(
                "/query",
                format!("a query is 1–{MAX_QUERY} bytes"),
            ));
        }
        if self.columns.is_empty() || self.columns.len() > MAX_COLUMNS {
            out.push(Refusal::new(
                "/column",
                format!("a loader declares 1–{MAX_COLUMNS} columns"),
            ));
        }
        for (i, col) in self.columns.iter().enumerate() {
            let at = format!("/column/{i}/name");
            if !is_identifier(&col.name) {
                out.push(Refusal::new(
                    at,
                    format!("column names match [a-z][a-z0-9_]* (got {:?})", col.name),
                ));
            } else if self.columns[..i].iter().any(|c| c.name == col.name) {
                out.push(Refusal::new(
                    at,
                    format!("column {:?} is declared twice", col.name),
                ));
            }
        }
    }

    /// What review approves, as one string. Every field is in it — a changed
    /// schedule changes how often private data leaves, so it is a different
    /// loader too.
    pub fn digest(&self) -> String {
        use sha2::Digest;
        #[derive(Serialize)]
        struct Canonical<'a> {
            name: &'a str,
            #[serde(flatten)]
            loader: &'a Loader,
        }
        let json = serde_json::to_string(&Canonical {
            name: &self.name,
            loader: self,
        })
        .expect("a loader always serializes");
        sha2::Sha256::digest(json.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Check one refresh's output against the declared shape and normalise it
    /// (timestamps to RFC 3339 UTC, 0/1 to booleans). `names` are the result's
    /// column names in order; each row has one value per name.
    ///
    /// `null` is accepted in every column: unknown is a value, and drawing it
    /// as zero is the renderer's mistake to avoid, not the loader's to hide.
    pub fn shape(
        &self,
        names: &[String],
        rows: Vec<Vec<Value>>,
    ) -> Result<Vec<Vec<Value>>, ShapeRefusal> {
        let declared: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
        if names
            .iter()
            .map(String::as_str)
            .ne(declared.iter().copied())
        {
            return Err(ShapeRefusal::Columns {
                declared: declared.iter().map(|s| s.to_string()).collect(),
                returned: names.to_vec(),
            });
        }
        if rows.len() > self.max_rows as usize {
            return Err(ShapeRefusal::TooManyRows { max: self.max_rows });
        }
        let mut shaped = Vec::with_capacity(rows.len());
        for (r, row) in rows.into_iter().enumerate() {
            if row.len() != self.columns.len() {
                return Err(ShapeRefusal::RowWidth {
                    row: r,
                    expected: self.columns.len(),
                    got: row.len(),
                });
            }
            let mut out = Vec::with_capacity(row.len());
            for (value, col) in row.into_iter().zip(&self.columns) {
                match coerce(value, col.kind) {
                    Some(v) => out.push(v),
                    None => {
                        return Err(ShapeRefusal::Value {
                            row: r,
                            column: col.name.clone(),
                            expected: col.kind,
                        })
                    }
                }
            }
            shaped.push(out);
        }
        Ok(shaped)
    }
}

/// Why a refresh's output was refused. Positions and types only — never the
/// value found, which may be third-party text (module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeRefusal {
    Columns {
        declared: Vec<String>,
        returned: Vec<String>,
    },
    TooManyRows {
        max: u32,
    },
    RowWidth {
        row: usize,
        expected: usize,
        got: usize,
    },
    Value {
        row: usize,
        column: String,
        expected: ColumnType,
    },
}

impl std::fmt::Display for ShapeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapeRefusal::Columns { declared, returned } => write!(
                f,
                "the query returned columns [{}] but the loader declares [{}]",
                returned.join(", "),
                declared.join(", ")
            ),
            ShapeRefusal::TooManyRows { max } => {
                write!(f, "the query returned more than max_rows ({max}) rows")
            }
            ShapeRefusal::RowWidth { row, expected, got } => {
                write!(
                    f,
                    "row {row} has {got} values; the loader declares {expected} columns"
                )
            }
            ShapeRefusal::Value {
                row,
                column,
                expected,
            } => write!(
                f,
                "row {row}, column {column:?}: the value is not a {}",
                expected.as_str()
            ),
        }
    }
}

impl std::error::Error for ShapeRefusal {}

fn coerce(value: Value, kind: ColumnType) -> Option<Value> {
    if value.is_null() {
        return Some(Value::Null);
    }
    match kind {
        ColumnType::String => value.is_string().then_some(value),
        ColumnType::Integer => match &value {
            Value::Number(n) if n.is_i64() || n.is_u64() => Some(value),
            Value::Number(n) => n
                .as_f64()
                .filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15)
                .map(|f| Value::from(f as i64)),
            _ => None,
        },
        ColumnType::Float => value.is_number().then_some(value),
        ColumnType::Boolean => match &value {
            Value::Bool(_) => Some(value),
            Value::Number(n) => match n.as_i64() {
                Some(0) => Some(Value::Bool(false)),
                Some(1) => Some(Value::Bool(true)),
                _ => None,
            },
            _ => None,
        },
        ColumnType::Date => value
            .as_str()
            .filter(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok())
            .map(|_| value.clone()),
        ColumnType::Timestamp => value.as_str().and_then(timestamp).map(Value::String),
    }
}

/// RFC 3339 with any offset, or SQLite's zone-less `YYYY-MM-DD HH:MM:SS[.f]`
/// (what `datetime('now')` writes, which is UTC). Always returned as UTC.
fn timestamp(s: &str) -> Option<String> {
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc).to_rfc3339());
    }
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"]
        .iter()
        .find_map(|fmt| NaiveDateTime::parse_from_str(s, fmt).ok())
        .map(|t| t.and_utc().to_rfc3339())
}
