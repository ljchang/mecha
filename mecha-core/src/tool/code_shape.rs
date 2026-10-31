//! The shape a sub-call's answer must have (`CODE-MODE-DESIGN.md` §4): a
//! program spells it, [`Shape::parse`] reads it before any pass is sent, it
//! compiles to the JSON Schema the pass is constrained by
//! ([`Shape::schema`]), and the whole answer is checked again here
//! ([`Shape::check`]), the way `subagent::AnswerShape::matches` checks a
//! child's. Decoder and parser both, so an endpoint that loosens its grammar
//! still cannot hand a program a value outside the shape.
//!
//! The spellings, as a program writes them (every one a JSON value, since a
//! Python type object such as a bare `bool` does not cross into the host):
//!
//! | Shape | Spelled | Bound |
//! |---|---|---|
//! | boolean | `"bool"` | |
//! | number | `"int"`, `"float"`, `"int[0,100]"`, `"float[0,1]"` | the range, where given |
//! | one of | `["urgent", "fyi", "none"]` | ≤ 32 labels, each 1–64 characters |
//! | list | `[shape]`: one element that is itself a shape | ≤ 64 items |
//! | record | `{"due": "str[32]", "urgent": "bool"}` | ≤ 8 fields, each a leaf |
//! | short text | `"str[n]"` | 1 ≤ n ≤ 2,000 |
//!
//! **One element, two readings.** A one-element list whose element is a
//! shape (`["bool"]`, `[{"x": "int"}]`) is a list; any other list of strings
//! is a one-of. A one-of of a single label that is also a shape keyword
//! cannot be spelled, and needs none: it has only one answer.
//!
//! **No nesting past one level**: a list's item is a leaf or a record, and
//! a record's fields are leaves (bool, a number, a one-of, `str[n]`).
//!
//! **`str[n]` is program-internal.** A length cap vouches for nothing ("ignore
//! previous instructions" is short), so nothing here converts to
//! `subagent::AnswerShape`, which deliberately has no text shape; a test
//! holds that `"str[32]"` does not parse as one.
//!
//! Every message here names what failed — a type, a missing field, a label
//! outside the set, an index — and never quotes the answer: the answer was
//! written by a model reading a stranger's bytes (§3).

use serde_json::{json, Map, Value};

/// The most labels a one-of may have.
pub const MAX_LABELS: usize = 32;
/// The longest label, and the longest record field name, in characters.
pub const MAX_LABEL_CHARS: usize = 64;
/// The most items a list may have.
pub const MAX_ITEMS: usize = 64;
/// The most fields a record may have.
pub const MAX_FIELDS: usize = 8;
/// The largest `n` in `str[n]`.
pub const MAX_TEXT: usize = 2000;

/// The answer's one field: every pass answers `{"answer": …}`, because a
/// strict endpoint wants an object at the top of a schema.
pub const ANSWER: &str = "answer";

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Bool,
    Int {
        min: Option<i64>,
        max: Option<i64>,
    },
    Float {
        min: Option<f64>,
        max: Option<f64>,
    },
    OneOf(Vec<String>),
    /// `str[n]`: at most `n` characters.
    Text(usize),
    List(Box<Shape>),
    /// Fields in the order the program wrote them: serde_json keeps an
    /// object's order here because the workspace enables `preserve_order`
    /// (root `Cargo.toml`), and the schema's `required` follows it.
    Record(Vec<(String, Shape)>),
}

/// Where in a shape a spelling sits, for the parse error.
fn at(path: &str) -> String {
    if path.is_empty() {
        "the shape".to_string()
    } else {
        format!("the shape at {path}")
    }
}

impl Shape {
    /// Read a program's spelling. `Err` says what is wrong with it, by
    /// position; it never quotes a string the program passed.
    pub fn parse(v: &Value) -> Result<Shape, String> {
        Self::parse_at(v, "", 0)
    }

    /// `depth` 0 is the top; a list's item and a record's field are depth 1,
    /// and nothing goes deeper but a record's fields under a list (depth 2,
    /// leaves only).
    fn parse_at(v: &Value, path: &str, depth: u8) -> Result<Shape, String> {
        match v {
            Value::String(s) => Self::keyword(s).ok_or_else(|| {
                format!(
                    "{} is not a shape: a string shape is bool, int, float, int[a,b], \
                     float[a,b] or str[n] (n from 1 to {MAX_TEXT})",
                    at(path)
                )
            }),
            Value::Array(items) => {
                if let [only] = items.as_slice() {
                    let is_shape = match only {
                        Value::String(s) => Self::keyword(s).is_some(),
                        Value::Object(_) | Value::Array(_) => true,
                        _ => false,
                    };
                    if is_shape {
                        if depth > 0 {
                            return Err(format!(
                                "{} is a list inside a list or a record; shapes nest one \
                                 level only",
                                at(path)
                            ));
                        }
                        let item = Self::parse_at(only, &format!("{path}[0]"), depth + 1)?;
                        if matches!(item, Shape::List(_)) {
                            return Err(format!(
                                "{} is a list of lists; shapes nest one level only",
                                at(path)
                            ));
                        }
                        return Ok(Shape::List(Box::new(item)));
                    }
                }
                Self::one_of(items, path)
            }
            // A record under a list is depth 1; one inside a record is
            // refused by `record`, and anything deeper here.
            Value::Object(_) if depth > 1 => Err(format!(
                "{} is a record inside a record; shapes nest one level only",
                at(path)
            )),
            Value::Object(fields) => Self::record(fields, path, depth),
            _ => Err(format!(
                "{} must be a string, a list or a dict (a bare Python type such as bool \
                 does not cross; spell it \"bool\")",
                at(path)
            )),
        }
    }

    fn one_of(items: &[Value], path: &str) -> Result<Shape, String> {
        if items.is_empty() || items.len() > MAX_LABELS {
            return Err(format!(
                "{} is a one-of with {} labels; it takes 1 to {MAX_LABELS}",
                at(path),
                items.len()
            ));
        }
        let mut labels = Vec::with_capacity(items.len());
        for (i, l) in items.iter().enumerate() {
            let Some(s) = l.as_str() else {
                return Err(format!(
                    "{}: label {i} is not a string (a list of one shape is a list; any \
                     other list is a one-of of strings)",
                    at(path)
                ));
            };
            let n = s.chars().count();
            if n == 0 || n > MAX_LABEL_CHARS {
                return Err(format!(
                    "{}: label {i} has {n} characters; a label has 1 to {MAX_LABEL_CHARS}",
                    at(path)
                ));
            }
            if labels.iter().any(|x: &String| x == s) {
                return Err(format!("{}: label {i} repeats an earlier one", at(path)));
            }
            labels.push(s.to_string());
        }
        Ok(Shape::OneOf(labels))
    }

    fn record(fields: &Map<String, Value>, path: &str, depth: u8) -> Result<Shape, String> {
        if fields.is_empty() || fields.len() > MAX_FIELDS {
            return Err(format!(
                "{} is a record with {} fields; it takes 1 to {MAX_FIELDS}",
                at(path),
                fields.len()
            ));
        }
        let mut out = Vec::with_capacity(fields.len());
        for (k, (name, v)) in fields.iter().enumerate() {
            let n = name.chars().count();
            if n == 0 || n > MAX_LABEL_CHARS {
                return Err(format!(
                    "{}: field {k}'s name has {n} characters; a name has 1 to \
                     {MAX_LABEL_CHARS}",
                    at(path)
                ));
            }
            let field_path = format!("{path}{{{name}}}");
            let shape = Self::parse_at(v, &field_path, depth + 1)?;
            if matches!(shape, Shape::List(_) | Shape::Record(_)) {
                return Err(format!(
                    "{} is a list or a record inside a record; a record's fields are \
                     bool, a number, a one-of or str[n]",
                    at(&field_path)
                ));
            }
            out.push((name.clone(), shape));
        }
        Ok(Shape::Record(out))
    }

    /// A string shape: `bool`, `int`, `float`, `int[a,b]`, `float[a,b]` or
    /// `str[n]`, spaces allowed inside the brackets.
    fn keyword(s: &str) -> Option<Shape> {
        let s = s.trim();
        match s {
            "bool" => return Some(Shape::Bool),
            "int" => {
                return Some(Shape::Int {
                    min: None,
                    max: None,
                })
            }
            "float" => {
                return Some(Shape::Float {
                    min: None,
                    max: None,
                })
            }
            _ => {}
        }
        let (head, rest) = s.split_once('[')?;
        let inner = rest.strip_suffix(']')?;
        match head.trim() {
            "str" => {
                let n: usize = inner.trim().parse().ok()?;
                (1..=MAX_TEXT).contains(&n).then_some(Shape::Text(n))
            }
            "int" => {
                let (a, b) = inner.split_once(',')?;
                let (a, b): (i64, i64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
                (a <= b).then_some(Shape::Int {
                    min: Some(a),
                    max: Some(b),
                })
            }
            "float" => {
                let (a, b) = inner.split_once(',')?;
                let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
                (a.is_finite() && b.is_finite() && a <= b).then_some(Shape::Float {
                    min: Some(a),
                    max: Some(b),
                })
            }
            _ => None,
        }
    }

    /// The JSON Schema for this shape's value.
    fn value_schema(&self) -> Value {
        match self {
            Shape::Bool => json!({"type": "boolean"}),
            Shape::Int { min, max } => {
                let mut s = json!({"type": "integer"});
                if let Some(m) = min {
                    s["minimum"] = json!(m);
                }
                if let Some(m) = max {
                    s["maximum"] = json!(m);
                }
                s
            }
            Shape::Float { min, max } => {
                let mut s = json!({"type": "number"});
                if let Some(m) = min {
                    s["minimum"] = json!(m);
                }
                if let Some(m) = max {
                    s["maximum"] = json!(m);
                }
                s
            }
            Shape::OneOf(labels) => json!({"type": "string", "enum": labels}),
            Shape::Text(n) => json!({"type": "string", "maxLength": n}),
            Shape::List(item) => json!({
                "type": "array",
                "items": item.value_schema(),
                "maxItems": MAX_ITEMS,
            }),
            Shape::Record(fields) => {
                let props: Map<String, Value> = fields
                    .iter()
                    .map(|(k, s)| (k.clone(), s.value_schema()))
                    .collect();
                let required: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
                json!({
                    "type": "object",
                    "properties": props,
                    "required": required,
                    "additionalProperties": false,
                })
            }
        }
    }

    /// The schema a pass is constrained by: `{"answer": <this shape>}`.
    pub fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {ANSWER: self.value_schema()},
            "required": [ANSWER],
            "additionalProperties": false,
        })
    }

    /// The most characters of text an answer of this shape can carry: what
    /// it could add to the `peek` cap at most, so a shape that cannot fit is
    /// refused before its pass (`[{"x": "str[2000]"}]` is 128,000).
    pub fn max_text_chars(&self) -> usize {
        match self {
            Shape::Text(n) => *n,
            Shape::List(item) => MAX_ITEMS.saturating_mul(item.max_text_chars()),
            Shape::Record(fields) => fields.iter().map(|(_, s)| s.max_text_chars()).sum(),
            _ => 0,
        }
    }

    /// The bytes of `str[n]` text in `v`, a value this shape already
    /// checked: what it adds to `peeked_bytes`. A one-of's label is the
    /// program's own word and is not counted.
    pub fn text_bytes(&self, v: &Value) -> usize {
        match (self, v) {
            (Shape::Text(_), Value::String(s)) => s.len(),
            (Shape::List(item), Value::Array(vs)) => vs.iter().map(|x| item.text_bytes(x)).sum(),
            (Shape::Record(fields), Value::Object(m)) => fields
                .iter()
                .map(|(k, s)| m.get(k).map_or(0, |x| s.text_bytes(x)))
                .sum(),
            _ => 0,
        }
    }

    /// Check a pass's whole content against this shape: it must be JSON,
    /// an object whose one field is `answer`, and the answer must have the
    /// shape. `Ok` is the answer's value; `Err` names what failed, never
    /// the text.
    pub fn check(&self, content: &str) -> Result<Value, String> {
        let Ok(v) = serde_json::from_str::<Value>(content.trim()) else {
            return Err("the answer is not JSON".into());
        };
        let Value::Object(mut top) = v else {
            return Err(format!(
                "the answer is not an object with the one field `{ANSWER}`"
            ));
        };
        if top.len() != 1 || !top.contains_key(ANSWER) {
            return Err(format!(
                "the answer is not an object with the one field `{ANSWER}` (it has {} \
                 fields)",
                top.len()
            ));
        }
        let answer = top.remove(ANSWER).unwrap_or(Value::Null);
        self.check_value(&answer, ANSWER)?;
        Ok(answer)
    }

    fn check_value(&self, v: &Value, path: &str) -> Result<(), String> {
        let wrong = |expected: &str| format!("wrong type at {path} (expected {expected})");
        match self {
            Shape::Bool => v
                .is_boolean()
                .then_some(())
                .ok_or_else(|| wrong("a boolean")),
            Shape::Int { min, max } => {
                let n = v.as_i64().ok_or_else(|| wrong("an integer"))?;
                if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                    return Err(format!("a number outside the shape's range at {path}"));
                }
                Ok(())
            }
            Shape::Float { min, max } => {
                let n = v.as_f64().ok_or_else(|| wrong("a number"))?;
                if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                    return Err(format!("a number outside the shape's range at {path}"));
                }
                Ok(())
            }
            Shape::OneOf(labels) => {
                let s = v.as_str().ok_or_else(|| wrong("one of the labels"))?;
                if labels.iter().any(|l| l == s) {
                    Ok(())
                } else {
                    Err(format!("a label outside the set at {path}"))
                }
            }
            Shape::Text(n) => {
                let s = v.as_str().ok_or_else(|| wrong("a string"))?;
                let len = s.chars().count();
                if len > *n {
                    return Err(format!(
                        "text of {len} characters at {path}, past the shape's {n}"
                    ));
                }
                Ok(())
            }
            Shape::List(item) => {
                let vs = v.as_array().ok_or_else(|| wrong("a list"))?;
                if vs.len() > MAX_ITEMS {
                    return Err(format!(
                        "a list of {} items at {path}, past {MAX_ITEMS}",
                        vs.len()
                    ));
                }
                for (i, x) in vs.iter().enumerate() {
                    item.check_value(x, &format!("{path}[{i}]"))?;
                }
                Ok(())
            }
            Shape::Record(fields) => {
                let m = v.as_object().ok_or_else(|| wrong("a record"))?;
                for (k, s) in fields {
                    let Some(x) = m.get(k) else {
                        return Err(format!("a missing field `{k}` at {path}"));
                    };
                    s.check_value(x, &format!("{path}.{k}"))?;
                }
                // Counted, never named: an extra field's name is the
                // model's text.
                let extra = m
                    .keys()
                    .filter(|k| !fields.iter().any(|(f, _)| f == *k))
                    .count();
                if extra > 0 {
                    return Err(format!(
                        "{extra} field(s) the shape does not name at {path}"
                    ));
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: Value) -> Shape {
        Shape::parse(&v).unwrap_or_else(|e| panic!("{v}: {e}"))
    }

    #[test]
    fn every_spelling_in_the_table_parses() {
        assert_eq!(parse(json!("bool")), Shape::Bool);
        assert_eq!(
            parse(json!("int")),
            Shape::Int {
                min: None,
                max: None
            }
        );
        assert_eq!(
            parse(json!("int[0, 100]")),
            Shape::Int {
                min: Some(0),
                max: Some(100)
            }
        );
        assert_eq!(
            parse(json!("float[-1,1]")),
            Shape::Float {
                min: Some(-1.0),
                max: Some(1.0)
            }
        );
        assert_eq!(parse(json!("str[32]")), Shape::Text(32));
        assert_eq!(
            parse(json!(["urgent", "fyi", "none"])),
            Shape::OneOf(vec!["urgent".into(), "fyi".into(), "none".into()])
        );
        assert_eq!(parse(json!(["bool"])), Shape::List(Box::new(Shape::Bool)));
        assert_eq!(
            parse(json!(["urgent"])),
            Shape::OneOf(vec!["urgent".into()]),
            "one label that is not a shape is a one-of"
        );
        assert_eq!(
            parse(json!({"due": "str[32]", "urgent": "bool"})),
            Shape::Record(vec![
                ("due".into(), Shape::Text(32)),
                ("urgent".into(), Shape::Bool)
            ])
        );
        assert!(matches!(
            parse(json!([{"x": "str[20]", "k": ["a", "b"]}])),
            Shape::List(item) if matches!(*item, Shape::Record(_))
        ));
    }

    /// Each bound in the table, one past it, and each nesting past one level.
    #[test]
    fn a_shape_past_its_bounds_or_nested_too_deep_is_refused() {
        let labels = |n: usize| Value::from((0..n).map(|i| format!("l{i}")).collect::<Vec<_>>());
        let fields =
            |n: usize| Value::Object((0..n).map(|i| (format!("f{i}"), json!("bool"))).collect());
        assert!(Shape::parse(&labels(MAX_LABELS)).is_ok());
        assert!(Shape::parse(&fields(MAX_FIELDS)).is_ok());
        assert!(Shape::parse(&json!("str[2000]")).is_ok());
        for bad in [
            labels(MAX_LABELS + 1),
            json!([]),
            json!(["a", "a"]),
            json!(["x".repeat(MAX_LABEL_CHARS + 1), "b"]),
            fields(MAX_FIELDS + 1),
            json!({}),
            json!("str[2001]"),
            json!("str[0]"),
            json!("int[5,1]"),
            json!("float[0,inf]"),
            json!("string"),
            json!(true),
            json!(3),
            json!([["bool"]]),
            json!({"a": {"b": "bool"}}),
            json!({"a": ["bool"]}),
            json!([{"a": {"b": "int"}}]),
            json!(["a", 1]),
        ] {
            assert!(Shape::parse(&bad).is_err(), "{bad} parsed");
        }
    }

    /// A parse error names the position, never the program's string.
    #[test]
    fn a_parse_error_quotes_nothing() {
        let err = Shape::parse(&json!({"x": "MARKER-7731"})).unwrap_err();
        assert!(!err.contains("MARKER-7731"), "{err}");
        assert!(err.contains("{x}"), "{err}");
    }

    #[test]
    fn the_schema_wraps_the_answer_and_bounds_each_shape() {
        let s = parse(json!([{"due": "str[32]", "urgent": "bool", "p": "int[0,3]"}])).schema();
        assert_eq!(s["required"], json!(["answer"]));
        assert_eq!(s["additionalProperties"], json!(false));
        let list = &s["properties"]["answer"];
        assert_eq!(list["type"], "array");
        assert_eq!(list["maxItems"], 64);
        let rec = &list["items"];
        assert_eq!(rec["additionalProperties"], json!(false));
        assert_eq!(rec["required"], json!(["due", "urgent", "p"]));
        assert_eq!(rec["properties"]["due"]["maxLength"], 32);
        assert_eq!(rec["properties"]["p"]["maximum"], 3);
        let e = parse(json!(["a", "b"])).schema();
        assert_eq!(e["properties"]["answer"]["enum"], json!(["a", "b"]));
    }

    /// The whole answer, decoder and parser both: anything but exactly the
    /// shape fails, and the failure names what, never the text.
    #[test]
    fn the_whole_answer_is_checked_and_a_failure_quotes_nothing() {
        let rec = parse(json!({"due": "str[8]", "urgent": "bool"}));
        assert_eq!(
            rec.check(r#"{"answer": {"due": "friday", "urgent": true}}"#)
                .unwrap(),
            json!({"due": "friday", "urgent": true})
        );
        let m = "MARKER-7731";
        for (bad, says) in [
            (format!("{m} not json"), "not JSON"),
            (format!(r#"{{"answer": true, "{m}": 1}}"#), "one field"),
            (
                format!(r#"{{"answer": {{"due": "{m}", "urgent": true}}}}"#),
                "past the shape's 8",
            ),
            (
                format!(r#"{{"answer": {{"due": "x", "urgent": "{m}"}}}}"#),
                "expected a boolean",
            ),
            (
                r#"{"answer": {"due": "x"}}"#.to_string(),
                "missing field `urgent`",
            ),
            (
                format!(r#"{{"answer": {{"due": "x", "urgent": true, "{m}": 1}}}}"#),
                "1 field(s) the shape does not name",
            ),
        ] {
            let err = rec.check(&bad).unwrap_err();
            assert!(err.contains(says), "{bad}: {err}");
            assert!(!err.contains(m), "{err}");
        }
        let one = parse(json!(["urgent", "fyi"]));
        assert!(one.check(r#"{"answer": "fyi"}"#).is_ok());
        let err = one.check(r#"{"answer": "MARKER-7731"}"#).unwrap_err();
        assert!(err.contains("outside the set") && !err.contains(m), "{err}");
        let range = parse(json!("int[0,10]"));
        assert!(range.check(r#"{"answer": 11}"#).is_err());
        assert!(range.check(r#"{"answer": 2.5}"#).is_err());
        assert!(range.check(r#"{"answer": 10}"#).is_ok());
        let list = parse(json!(["bool"]));
        let long = format!(r#"{{"answer": [{}]}}"#, vec!["true"; 65].join(","));
        assert!(list.check(&long).unwrap_err().contains("65 items"));
    }

    #[test]
    fn text_is_counted_by_its_bytes_and_bounded_up_front() {
        let s = parse(json!([{"x": "str[2000]", "k": ["a", "b"]}]));
        assert_eq!(s.max_text_chars(), 128_000, "the design's example");
        let v = json!([{"x": "héllo", "k": "a"}, {"x": "ab", "k": "b"}]);
        assert_eq!(s.text_bytes(&v), 6 + 2, "labels are not counted");
        assert_eq!(parse(json!(["a", "b"])).max_text_chars(), 0);
    }

    /// `str[n]` never becomes a boundary shape: `trusted_output`'s
    /// `AnswerShape` has no text shape, and a sub-call's spelling of one
    /// does not parse as it.
    #[test]
    fn a_text_shape_is_never_an_answer_shape() {
        for s in ["str[32]", "bool", "int"] {
            assert!(
                serde_json::from_value::<crate::subagent::AnswerShape>(json!(s)).is_err(),
                "{s} parsed as a trusted_output shape"
            );
        }
    }
}
