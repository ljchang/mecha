//! The Vega-Lite subset a chart panel may use (design §2.2).
//!
//! Vega-Lite's published schema is enormous and permissive, and the question
//! here is not "is this valid Vega-Lite" — the renderer's compile answers
//! that — but "does it use only what we allow". So the walk is an allowlist
//! over the *structure*: the keys of a view, the data reference, the mark
//! types, the encoding channels, the keys of a field definition, the
//! transform operations, the keys of a parameter. Anything unknown there is
//! refused, so a Vega-Lite release that adds a destination-bearing field to
//! one of those cannot slip through.
//!
//! The *style* objects beneath them — `axis`, `legend`, `scale`, a mark's
//! properties, a title's — run to hundreds of presentational keys, and
//! allowlisting them would be a copy of Vega-Lite's schema that drifts.
//! Those are screened instead: any key that names a link, a URL, a source or
//! a loader is refused, and every string in the whole spec has already been
//! screened for addresses by `spec::screen_strings`. Between the two, a style
//! object can change how a chart looks and cannot make it fetch or navigate.
//!
//! **Expressions live in named places only**: a `filter` or `calculate`
//! transform, a parameter's `expr`, a condition's `test`. Vega-Lite also lets
//! almost any presentational property be `{"expr": ...}`, and some take a bare
//! expression string under a key ending in `Expr` (`axis.labelExpr`); both
//! pass the address and destination screens, so inside a style object an
//! `expr` key, and any key ending in `Expr`, is refused. Styling that
//! depends on data goes through an encoding's `condition`; the rest comes
//! from the owner's theme.
//!
//! One place carries expressions without being named above, on purpose: a
//! parameter's `select` takes Vega event streams (`on`, `clear`, `translate`,
//! `zoom`), whose bracketed filters are expressions. They are screened as
//! style — no destination key, no address — and evaluated by the same
//! interpreter, whose language cannot fetch or navigate.

use serde_json::{Map, Value};

use super::{pointer, Refusal};

const MARKS: &[&str] = &[
    "arc",
    "area",
    "bar",
    "boxplot",
    "circle",
    "errorband",
    "errorbar",
    "line",
    "point",
    "rect",
    "rule",
    "square",
    "text",
    "tick",
    "trail",
];

const CHANNELS: &[&str] = &[
    "x",
    "y",
    "x2",
    "y2",
    "xOffset",
    "yOffset",
    "xError",
    "xError2",
    "yError",
    "yError2",
    "theta",
    "theta2",
    "radius",
    "radius2",
    "color",
    "fill",
    "stroke",
    "opacity",
    "fillOpacity",
    "strokeOpacity",
    "strokeWidth",
    "strokeDash",
    "size",
    "angle",
    "shape",
    "text",
    "tooltip",
    "detail",
    "key",
    "order",
    "row",
    "column",
    "facet",
    "description",
];

/// Keys of a field definition that are plain style objects.
const FIELD_DEF_STYLE: &[&str] = &[
    "axis",
    "legend",
    "scale",
    "header",
    "bin",
    "sort",
    "impute",
    "title",
    "stack",
    "timeUnit",
    "aggregate",
    "format",
    "value",
    "datum",
    "align",
    "center",
    "spacing",
    "band",
];

/// Each transform operation and the keys its object may carry beside the
/// operation's own. Vega-Lite tells its transforms apart by which key is
/// present, so an unknown sibling key is not inert — it may be a future
/// operation — and is refused rather than screened.
const TRANSFORMS: &[(&str, &[&str])] = &[
    ("filter", &[]),
    ("calculate", &["as"]),
    ("aggregate", &["groupby"]),
    ("joinaggregate", &["groupby"]),
    ("fold", &["as"]),
    ("window", &["frame", "ignorePeers", "groupby", "sort"]),
    ("bin", &["field", "as"]),
    ("timeUnit", &["field", "as"]),
    ("stack", &["groupby", "offset", "sort", "as"]),
    ("flatten", &["as"]),
    ("pivot", &["value", "groupby", "limit", "op"]),
    (
        "density",
        &[
            "groupby",
            "cumulative",
            "counts",
            "bandwidth",
            "extent",
            "minsteps",
            "maxsteps",
            "steps",
            "as",
        ],
    ),
    (
        "regression",
        &["on", "groupby", "method", "order", "extent", "params", "as"],
    ),
    ("loess", &["on", "groupby", "bandwidth", "as"]),
    ("quantile", &["groupby", "probs", "step", "as"]),
    (
        "impute",
        &["key", "keyvals", "frame", "method", "value", "groupby"],
    ),
    ("extent", &["param"]),
    ("sample", &[]),
];

const PARAM_KEYS: &[&str] = &["name", "select", "value", "expr", "views"];

/// Fragments of a key that, anywhere in a style object, name somewhere.
const DESTINATION_KEYS: &[&str] = &["url", "href", "src", "link", "loader", "usermeta"];

pub(super) fn check(v: &Value, datasets: &[String], at: &str, out: &mut Vec<Refusal>) {
    view(v, datasets, at, out);
    // A layer or concat child may inherit its parent's data, so no single view
    // must name one — but a chart that binds nothing anywhere is an empty
    // panel, which is a mistake to refuse rather than a picture to draw.
    if v.is_object() && !binds_data(v) {
        out.push(Refusal::new(
            at,
            "the chart binds no dataset: give it \"data\": {\"name\": <dataset>}",
        ));
    }
}

/// Only view positions count — the view itself, `spec`, and the composition
/// arrays — so a `data` key inside a style object cannot satisfy the check.
fn binds_data(v: &Value) -> bool {
    let Some(map) = v.as_object() else {
        return false;
    };
    map.contains_key("data")
        || map.get("spec").is_some_and(binds_data)
        || ["layer", "hconcat", "vconcat", "concat"].iter().any(|k| {
            map.get(*k)
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(binds_data))
        })
}

fn view(v: &Value, datasets: &[String], at: &str, out: &mut Vec<Refusal>) {
    let Some(map) = v.as_object() else {
        out.push(Refusal::new(at, "a Vega-Lite view is a JSON object"));
        return;
    };
    for (key, val) in map {
        let here = pointer(at, key);
        match key.as_str() {
            "$schema" if !at.ends_with("/vegalite") => out.push(Refusal::new(
                here,
                "`$schema` belongs only at the chart's top level",
            )),
            "$schema" | "description" | "name" => {
                if !val.is_string() {
                    out.push(Refusal::new(here, "expected a string"));
                }
            }
            "title" | "width" | "height" | "autosize" | "padding" | "background" | "bounds"
            | "align" | "center" | "spacing" | "columns" | "resolve" | "repeat" => {
                style(val, &here, out)
            }
            "data" => data(val, datasets, &here, out),
            "mark" => mark(val, &here, out),
            "encoding" => encoding(val, &here, out),
            "transform" => each(val, &here, out, transform),
            "params" => each(val, &here, out, param),
            "layer" | "hconcat" | "vconcat" | "concat" => each(val, &here, out, |item, at, out| {
                view(item, datasets, at, out)
            }),
            "spec" => view(val, datasets, &here, out),
            "facet" => facet(val, &here, out),
            "datasets" => out.push(Refusal::new(
                here,
                "inline `datasets` bypass the loader's reviewed shape; bind to a dataset with \
                 \"data\": {\"name\": ...}",
            )),
            "config" => out.push(Refusal::new(
                here,
                "`config` comes from the owner's theme, not the spec — remove it",
            )),
            "usermeta" => out.push(Refusal::new(here, "`usermeta` is not part of the subset")),
            "projection" => out.push(Refusal::new(
                here,
                "maps (`projection`) are not in the v1 subset",
            )),
            other => out.push(Refusal::new(
                here,
                format!("{other:?} is not a key the dashboard subset allows in a view"),
            )),
        }
    }
}

fn data(v: &Value, datasets: &[String], at: &str, out: &mut Vec<Refusal>) {
    let Some(map) = v.as_object() else {
        out.push(Refusal::new(at, "`data` is {\"name\": <dataset>}"));
        return;
    };
    if map.contains_key("url") {
        out.push(Refusal::new(
            pointer(at, "url"),
            "`data.url` fetches from an address the spec chooses; bind to a dataset with \
             {\"name\": <dataset>} instead",
        ));
    }
    if map.contains_key("values") {
        out.push(Refusal::new(
            pointer(at, "values"),
            "inline `data.values` bypass the loader's reviewed shape; bind to a dataset with \
             {\"name\": <dataset>} instead",
        ));
    }
    for key in map.keys() {
        if !matches!(key.as_str(), "name" | "url" | "values") {
            out.push(Refusal::new(pointer(at, key), "`data` takes only `name`"));
        }
    }
    match map.get("name") {
        Some(Value::String(name)) if datasets.iter().any(|d| d == name) => {}
        Some(Value::String(name)) => out.push(Refusal::new(
            pointer(at, "name"),
            format!(
                "dataset {name:?} is not in the spec's `datasets` ({})",
                datasets.join(", ")
            ),
        )),
        Some(_) => out.push(Refusal::new(pointer(at, "name"), "expected a dataset name")),
        None if !map.contains_key("url") && !map.contains_key("values") => {
            out.push(Refusal::new(at, "`data` needs a `name`"))
        }
        None => {}
    }
}

fn mark(v: &Value, at: &str, out: &mut Vec<Refusal>) {
    let (kind, at_kind) = match v {
        Value::String(s) => (s.as_str(), at.to_string()),
        Value::Object(map) => {
            for (key, val) in map {
                if key != "type" {
                    style_entry(key, val, at, out);
                }
            }
            match map.get("type").and_then(Value::as_str) {
                Some(s) => (s, pointer(at, "type")),
                None => {
                    out.push(Refusal::new(at, "a mark object needs a `type`"));
                    return;
                }
            }
        }
        _ => {
            out.push(Refusal::new(
                at,
                "a mark is a type name or an object with `type`",
            ));
            return;
        }
    };
    match kind {
        k if MARKS.contains(&k) => {}
        "image" => out.push(Refusal::new(
            at_kind,
            "the image mark loads a picture from a data-derived address; it is not in the subset",
        )),
        "geoshape" => out.push(Refusal::new(
            at_kind,
            "maps (`geoshape`) are not in the v1 subset",
        )),
        other => out.push(Refusal::new(
            at_kind,
            format!("mark {other:?} is not in the subset ({})", MARKS.join(", ")),
        )),
    }
}

fn encoding(v: &Value, at: &str, out: &mut Vec<Refusal>) {
    let Some(map) = v.as_object() else {
        out.push(Refusal::new(at, "`encoding` is an object of channels"));
        return;
    };
    for (channel, def) in map {
        let here = pointer(at, channel);
        match channel.as_str() {
            "href" | "url" => out.push(Refusal::new(
                here,
                format!(
                    "the {channel} channel turns a data value into an address to navigate or \
                     fetch; charts may not name destinations"
                ),
            )),
            c if CHANNELS.contains(&c) => match def {
                Value::Array(items) => {
                    for (i, item) in items.iter().enumerate() {
                        field_def(item, &format!("{here}/{i}"), out);
                    }
                }
                Value::Null => {}
                other => field_def(other, &here, out),
            },
            other => out.push(Refusal::new(
                here,
                format!("{other:?} is not an encoding channel the subset allows"),
            )),
        }
    }
}

fn field_def(v: &Value, at: &str, out: &mut Vec<Refusal>) {
    let Some(map) = v.as_object() else {
        out.push(Refusal::new(
            at,
            "a channel takes a field definition object",
        ));
        return;
    };
    for (key, val) in map {
        let here = pointer(at, key);
        match key.as_str() {
            "formatType" => out.push(Refusal::new(
                here,
                "`formatType` names a custom formatter function; use `format` with a d3 format \
                 string instead",
            )),
            "condition" => match val {
                Value::Array(items) => {
                    for (i, item) in items.iter().enumerate() {
                        field_def(item, &format!("{here}/{i}"), out);
                    }
                }
                other => field_def(other, &here, out),
            },
            // `field` ({"repeat": …}) and `test` (a predicate object) can hold
            // objects, so they are screened like style; the rest are scalars.
            "field" | "test" => style(val, &here, out),
            // Scalars in valid Vega-Lite, but screened all the same: `style`
            // is a no-op on a scalar, and an object here would otherwise be
            // the one nested value no screen reads.
            "type" | "param" | "empty" | "bandPosition" | "columns" => style(val, &here, out),
            k if FIELD_DEF_STYLE.contains(&k) => style(val, &here, out),
            k => out.push(Refusal::new(
                here,
                format!("{k:?} is not a field-definition key the subset allows"),
            )),
        }
    }
}

fn facet(v: &Value, at: &str, out: &mut Vec<Refusal>) {
    let Some(map) = v.as_object() else {
        out.push(Refusal::new(
            at,
            "`facet` is a field definition or {row, column}",
        ));
        return;
    };
    if map.contains_key("row") || map.contains_key("column") {
        for (key, val) in map {
            match key.as_str() {
                "row" | "column" => field_def(val, &pointer(at, key), out),
                other => out.push(Refusal::new(
                    pointer(at, other),
                    "`facet` takes `row` and `column`",
                )),
            }
        }
    } else {
        field_def(v, at, out);
    }
}

fn transform(v: &Value, at: &str, out: &mut Vec<Refusal>) {
    let Some(map) = v.as_object() else {
        out.push(Refusal::new(at, "a transform is an object"));
        return;
    };
    if map.contains_key("lookup") {
        out.push(Refusal::new(
            pointer(at, "lookup"),
            "`lookup` joins against another data source; join in the loader's query instead",
        ));
        return;
    }
    // Two covering ops would need A in B's option list and B in A's; the only
    // op name in any option list is `extent`, whose own list is just
    // `param`, so at most one op ever covers and the fallback always refuses.
    // The operation is the one whose key set covers every key present. More
    // than one known op key is legal only where one op's options name another
    // (`density` takes an `extent`), and the covering rule settles which.
    let covering: Vec<&(&str, &[&str])> = TRANSFORMS
        .iter()
        .filter(|(op, rest)| {
            map.contains_key(*op) && map.keys().all(|k| k == op || rest.contains(&k.as_str()))
        })
        .collect();
    debug_assert!(
        covering.len() <= 1,
        "two transform ops cover one object; the fallback arm would accept it unscreened"
    );
    match covering.as_slice() {
        // Every key is known to be allowed; their values are style.
        [_] => {
            for (key, val) in map {
                style(val, &pointer(at, key), out);
            }
        }
        _ => {
            let ops: Vec<&str> = TRANSFORMS.iter().map(|(op, _)| *op).collect();
            let present: Vec<&str> = map.keys().map(String::as_str).collect();
            match TRANSFORMS.iter().find(|(op, _)| map.contains_key(*op)) {
                Some((op, rest)) => {
                    for key in map
                        .keys()
                        .filter(|k| *k != op && !rest.contains(&k.as_str()))
                    {
                        out.push(Refusal::new(
                            pointer(at, key),
                            format!(
                                "{key:?} is not an option of the `{op}` transform (allowed: {})",
                                rest.join(", ")
                            ),
                        ));
                    }
                }
                None => out.push(Refusal::new(
                    at,
                    format!(
                        "no transform the subset allows here (found {}; allowed: {})",
                        present.join(", "),
                        ops.join(", ")
                    ),
                )),
            }
        }
    }
}

fn param(v: &Value, at: &str, out: &mut Vec<Refusal>) {
    let Some(map) = v.as_object() else {
        out.push(Refusal::new(at, "a parameter is an object"));
        return;
    };
    if !map.get("name").is_some_and(Value::is_string) {
        out.push(Refusal::new(at, "a parameter needs a `name`"));
    }
    for (key, val) in map {
        let here = pointer(at, key);
        match key.as_str() {
            "bind" => out.push(Refusal::new(
                here,
                "`bind` draws an input inside the chart; dashboard inputs are the spec's \
                 `filters`, which the renderer draws and links across panels",
            )),
            k if PARAM_KEYS.contains(&k) => style(val, &here, out),
            k => out.push(Refusal::new(
                here,
                format!("{k:?} is not a parameter key the subset allows"),
            )),
        }
    }
}

fn each(v: &Value, at: &str, out: &mut Vec<Refusal>, f: impl Fn(&Value, &str, &mut Vec<Refusal>)) {
    match v.as_array() {
        Some(items) => {
            for (i, item) in items.iter().enumerate() {
                f(item, &format!("{at}/{i}"), out);
            }
        }
        None => out.push(Refusal::new(at, "expected an array")),
    }
}

/// A presentational value: anything, as long as no key in it names a
/// destination.
fn style(v: &Value, at: &str, out: &mut Vec<Refusal>) {
    match v {
        Value::Object(map) => style_object(map, at, out),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                style(item, &format!("{at}/{i}"), out);
            }
        }
        _ => {}
    }
}

fn style_object(map: &Map<String, Value>, at: &str, out: &mut Vec<Refusal>) {
    for (key, val) in map {
        style_entry(key, val, at, out);
    }
}

fn style_entry(key: &str, val: &Value, at: &str, out: &mut Vec<Refusal>) {
    let here = pointer(at, key);
    if key == "expr" || key.ends_with("Expr") {
        out.push(Refusal::new(
            here,
            "expressions are allowed only in a filter or calculate transform, a parameter's \
             `expr` and a condition's `test`; style that depends on data goes through an \
             encoding's `condition`",
        ));
        return;
    }
    let lower = key.to_ascii_lowercase();
    if let Some(fragment) = DESTINATION_KEYS.iter().find(|f| lower.contains(*f)) {
        out.push(Refusal::new(
            here,
            format!("keys naming a {fragment} are not allowed anywhere in a chart"),
        ));
        return;
    }
    style(val, &here, out);
}

#[cfg(test)]
mod transform_table {
    use super::TRANSFORMS;

    /// The covering rule in `transform` assumes no two operations can both
    /// cover one object. In release builds the `debug_assert` there is gone,
    /// so pin the table itself: no op may list another op as an option while
    /// that op lists it back.
    #[test]
    fn no_two_transform_ops_cover_each_other() {
        for (a, a_opts) in TRANSFORMS {
            for (b, b_opts) in TRANSFORMS {
                if a != b {
                    assert!(
                        !(a_opts.contains(b) && b_opts.contains(a)),
                        "`{a}` and `{b}` each list the other as an option"
                    );
                }
            }
        }
    }
}
