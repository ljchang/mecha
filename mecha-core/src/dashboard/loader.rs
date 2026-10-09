//! A loader: a registered source, a query, a declared shape, and a schedule —
//! and the check every refresh's output must pass.
//!
//! "Read-only" is not checked here, and cannot be by reading the query: it is
//! enforced where the source is opened (step 2: a read-only open, no
//! `ATTACH`, one prepared statement — design §3.1). A `DROP TABLE` parses
//! as a loader and fails at the source.
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

use super::{is_identifier, Checked, Refusal, Refusals};
use crate::cron::Schedule;

/// Rows one dataset may hold. A dashboard draws pictures, not tables of
/// record; a query that needs more is aggregating in the wrong place.
pub const MAX_ROWS: u32 = 100_000;
const MAX_COLUMNS: usize = 64;
const MAX_QUERY: usize = 16 * 1024;
/// A dataset's size as published: the row cap bounds the count, this bounds
/// the bytes, since a string column has no length of its own.
pub const MAX_DATASET_BYTES: usize = 8 * 1024 * 1024;

/// A checked loader. Like [`super::Spec`]: not `Deserialize`, carrying a
/// private [`Checked`], fields private behind read-only accessors. The only
/// way to hold one is [`Loader::parse`], and nothing can change its query
/// afterwards — so [`Loader::digest`] always names the loader that was
/// checked, which is the one a human would release.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Loader {
    #[serde(skip)]
    checked: Checked,
    /// The file's stem, which is the dataset's name. Never read from the file:
    /// a name that can disagree with its filename is a bug with no upside.
    #[serde(skip)]
    name: String,
    /// A source the owner registered in `sources.toml`. Never a connection
    /// string — the model names a source, it never holds a credential.
    source: String,
    schedule: Schedule,
    /// IANA zone for the schedule; `None` is UTC. Never an offset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timezone: Option<String>,
    max_rows: u32,
    query: String,
    #[serde(rename = "column")]
    columns: Vec<Column>,
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

/// The wire shape of `loaders/<name>.toml`, before any check.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    source: String,
    schedule: Schedule,
    #[serde(default)]
    timezone: Option<String>,
    max_rows: u32,
    query: String,
    #[serde(rename = "column")]
    columns: Vec<Column>,
}

impl Loader {
    /// The dataset's name, from the file's stem.
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }
    pub fn timezone(&self) -> Option<&str> {
        self.timezone.as_deref()
    }
    pub fn max_rows(&self) -> u32 {
        self.max_rows
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Parse and check one `loaders/<name>.toml`.
    pub fn parse(name: &str, text: &str) -> Result<Loader, Refusals> {
        let wire: Wire = toml::from_str(text).map_err(|e| {
            Refusals(vec![Refusal::new(
                "",
                format!("does not match a loader's shape: {e}"),
            )])
        })?;
        let loader = Loader {
            checked: Checked::new(),
            name: name.to_string(),
            source: wire.source,
            schedule: wire.schedule,
            timezone: wire.timezone,
            max_rows: wire.max_rows,
            query: wire.query,
            columns: wire.columns,
        };
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
        let mut bytes = 0usize;
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
                    Some(v) => {
                        bytes += approx_len(&v);
                        if bytes > MAX_DATASET_BYTES {
                            return Err(ShapeRefusal::TooLarge {
                                max_bytes: MAX_DATASET_BYTES,
                            });
                        }
                        out.push(v)
                    }
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
    TooLarge {
        max_bytes: usize,
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
            ShapeRefusal::Columns { declared, returned } => {
                // A returned name may come from a schema a third party
                // controls; show it only when it is a plain identifier.
                let returned: Vec<&str> = returned
                    .iter()
                    .map(|n| {
                        if is_identifier(n) {
                            n.as_str()
                        } else {
                            "(unnamed)"
                        }
                    })
                    .collect();
                write!(
                    f,
                    "the query returned columns [{}] but the loader declares [{}]",
                    returned.join(", "),
                    declared.join(", ")
                )
            }
            ShapeRefusal::TooManyRows { max } => {
                write!(f, "the query returned more than max_rows ({max}) rows")
            }
            ShapeRefusal::TooLarge { max_bytes } => write!(
                f,
                "the dataset is larger than {max_bytes} bytes; aggregate in the query"
            ),
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

/// Roughly what a value costs serialized — exact for strings, near enough for
/// the rest, and never an allocation.
fn approx_len(v: &Value) -> usize {
    match v {
        Value::String(s) => s.len() + 2,
        Value::Null | Value::Bool(_) => 5,
        _ => 20,
    }
}

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
        ColumnType::Date => {
            let ok = value
                .as_str()
                .is_some_and(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok());
            ok.then_some(value)
        }
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
