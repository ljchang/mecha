//! A form over a TOML file the owner writes: typed fields the page draws, and
//! an edit that changes only the values it names.
//!
//! **One language describes the file.** The page never writes TOML: it sends
//! `{path: value}` changes, and [`apply`] sets each one in place with
//! `toml_edit`, so every comment, blank line and key order the owner wrote
//! survives — a comment trailing the very value being changed included. The
//! charter's editor learned the other way round what that costs: a
//! JavaScript serialiser that regenerates tables loses the comments among
//! them, and needs a second checker to keep it agreeing with the Rust reader.
//!
//! **Values are read through the file's own type.** [`values`] takes the
//! typed document as JSON (`serde_json::to_value(&settings)`), so a key the
//! file leaves out shows its serde default — nothing here guesses one.
//!
//! **The form is the fence.** [`apply`] refuses a path the form does not
//! declare and a value of the wrong kind, so the endpoint behind a form can
//! write exactly what the form shows and nothing else. Whether the result
//! *loads* is still the caller's reader's call: this module checks shape,
//! the type checks meaning.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{Map, Value as Json};
use toml_edit::{Array, DocumentMut, Item, Table, Value};

/// The most entries a chip field may hold.
pub const MAX_CHIPS: usize = 64;

/// A whole form: sections of fields, in the order the page draws them.
#[derive(Debug, Clone, Serialize)]
pub struct Form {
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Section {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Stored and not read by anything yet: the page draws it quieter, so a
    /// switch that is on never reads as working.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unbuilt: bool,
    pub fields: Vec<Field>,
}

/// One value in the file, by its dotted path (`safety.crisis`).
#[derive(Debug, Clone, Serialize)]
pub struct Field {
    pub path: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// As [`Section::unbuilt`], for one field.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unbuilt: bool,
    #[serde(flatten)]
    pub kind: Kind,
}

/// What a field holds, and so how the page draws it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    /// A boolean.
    Toggle,
    /// One line of text. With `optional`, an empty value removes the key.
    Text {
        max: usize,
        optional: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    /// One of a closed set. With `none`, that label offers leaving the key
    /// out (sent as `null`).
    Choice {
        options: Vec<Opt>,
        #[serde(skip_serializing_if = "Option::is_none")]
        none: Option<String>,
    },
    /// A list of names. With `free`, names beyond `options` may be typed.
    Chips { options: Vec<Opt>, free: bool },
}

#[derive(Debug, Clone, Serialize)]
pub struct Opt {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl Opt {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Opt {
        Opt {
            value: value.into(),
            label: label.into(),
            help: None,
        }
    }

    pub fn help(mut self, help: impl Into<String>) -> Opt {
        self.help = Some(help.into());
        self
    }

    /// Options whose label is their value — names from a store.
    pub fn names<S: AsRef<str>>(names: impl IntoIterator<Item = S>) -> Vec<Opt> {
        names
            .into_iter()
            .map(|n| Opt::new(n.as_ref(), n.as_ref()))
            .collect()
    }
}

impl Section {
    pub fn new(title: impl Into<String>) -> Section {
        Section {
            title: title.into(),
            help: None,
            unbuilt: false,
            fields: Vec::new(),
        }
    }

    pub fn help(mut self, help: impl Into<String>) -> Section {
        self.help = Some(help.into());
        self
    }

    pub fn unbuilt(mut self) -> Section {
        self.unbuilt = true;
        self
    }

    pub fn field(mut self, field: Field) -> Section {
        self.fields.push(field);
        self
    }
}

impl Field {
    pub fn new(path: impl Into<String>, label: impl Into<String>, kind: Kind) -> Field {
        Field {
            path: path.into(),
            label: label.into(),
            help: None,
            unbuilt: false,
            kind,
        }
    }

    pub fn unbuilt(mut self) -> Field {
        self.unbuilt = true;
        self
    }

    pub fn toggle(path: impl Into<String>, label: impl Into<String>) -> Field {
        Field::new(path, label, Kind::Toggle)
    }

    pub fn help(mut self, help: impl Into<String>) -> Field {
        self.help = Some(help.into());
        self
    }

    /// Is `v` a value this field may be set to? `Ok` carries it normalised
    /// (text trimmed, chips trimmed and de-duplicated in order).
    fn check(&self, v: &Json) -> Result<Json> {
        let path = &self.path;
        match (&self.kind, v) {
            (Kind::Toggle, Json::Bool(_)) => Ok(v.clone()),
            (Kind::Text { optional: true, .. }, Json::Null) => Ok(Json::Null),
            (Kind::Text { max, optional, .. }, Json::String(s)) => {
                let s = s.trim();
                if s.is_empty() && *optional {
                    return Ok(Json::Null);
                }
                if s.chars().count() > *max {
                    bail!("`{path}` is at most {max} characters");
                }
                if s.chars().any(char::is_control) {
                    bail!("`{path}` is one line, with no control characters");
                }
                Ok(Json::String(s.to_string()))
            }
            (Kind::Choice { none: Some(_), .. }, Json::Null) => Ok(Json::Null),
            (Kind::Choice { options, .. }, Json::String(s)) => {
                if options.iter().any(|o| o.value == *s) {
                    Ok(v.clone())
                } else {
                    bail!("`{s}` is not one of the choices for `{path}`")
                }
            }
            (Kind::Chips { options, free }, Json::Array(items)) => {
                if items.len() > MAX_CHIPS {
                    bail!("`{path}` holds at most {MAX_CHIPS} entries");
                }
                let mut out: Vec<String> = Vec::new();
                for item in items {
                    let Some(s) = item.as_str() else {
                        bail!("`{path}` is a list of names");
                    };
                    let s = s.trim();
                    if s.is_empty() || s.chars().any(char::is_control) {
                        bail!("`{path}` holds an empty or unprintable name");
                    }
                    if !free && !options.iter().any(|o| o.value == s) {
                        bail!("`{s}` is not one of the choices for `{path}`");
                    }
                    if !out.iter().any(|o| o == s) {
                        out.push(s.to_string());
                    }
                }
                Ok(Json::from(out))
            }
            _ => bail!("`{path}` cannot be set to {v}"),
        }
    }
}

impl Form {
    pub fn field(&self, path: &str) -> Option<&Field> {
        self.sections
            .iter()
            .flat_map(|s| &s.fields)
            .find(|f| f.path == path)
    }
}

/// Every field's current value, read out of the typed document as JSON. A
/// path the type does not carry reads `null`.
pub fn values(form: &Form, typed: &Json) -> Map<String, Json> {
    form.sections
        .iter()
        .flat_map(|s| &s.fields)
        .map(|f| {
            let v = f
                .path
                .split('.')
                .try_fold(typed, |at, seg| at.get(seg))
                .cloned()
                .unwrap_or(Json::Null);
            (f.path.clone(), v)
        })
        .collect()
}

/// `text` with each change set in place. Every path must be a field of
/// `form` and every value one it may hold; otherwise nothing is changed and
/// the first refusal is the error. `null` removes a key.
pub fn apply(form: &Form, text: &str, changes: &Map<String, Json>) -> Result<String> {
    let mut doc: DocumentMut = text
        .parse()
        .context("the file is not valid TOML, so the form cannot edit it — edit it as text")?;
    let mut checked = Vec::with_capacity(changes.len());
    for (path, v) in changes {
        let Some(field) = form.field(path) else {
            bail!("`{path}` is not a field of this form");
        };
        checked.push((path, field.check(v)?));
    }
    for (path, v) in checked {
        set(doc.as_table_mut(), path, &v)?;
    }
    Ok(doc.to_string())
}

fn set(root: &mut Table, path: &str, v: &Json) -> Result<()> {
    let segs: Vec<&str> = path.split('.').collect();
    let (leaf, parents) = segs.split_last().context("an empty path")?;
    let mut table: &mut dyn toml_edit::TableLike = root;
    for seg in parents {
        if table.get(seg).is_none() {
            if v.is_null() {
                // Removing a key under a table that is not there: done.
                return Ok(());
            }
            table.insert(seg, Item::Table(Table::new()));
        }
        table = table
            .get_mut(seg)
            .and_then(Item::as_table_like_mut)
            .with_context(|| format!("`{seg}` in `{path}` is not a table in the file"))?;
    }
    if v.is_null() {
        table.remove(leaf);
        return Ok(());
    }
    let mut new = to_toml(v)?;
    // Keep the old value's decor — a comment trailing it on its line is the
    // owner's, and it stays beside the value it described.
    if let Some(old) = table.get(leaf).and_then(Item::as_value) {
        *new.decor_mut() = old.decor().clone();
    }
    match table.get_mut(leaf) {
        Some(item) => *item = Item::Value(new),
        None => {
            table.insert(leaf, Item::Value(new));
        }
    }
    Ok(())
}

fn to_toml(v: &Json) -> Result<Value> {
    Ok(match v {
        Json::Bool(b) => Value::from(*b),
        Json::String(s) => Value::from(s.as_str()),
        Json::Array(items) => {
            let mut a = Array::new();
            for i in items {
                a.push(i.as_str().context("a list of names")?);
            }
            Value::Array(a)
        }
        other => bail!("{other} has no TOML form here"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn form() -> Form {
        Form {
            sections: vec![
                Section::new("Who")
                    .field(Field::new(
                        "display",
                        "Name",
                        Kind::Text {
                            max: 40,
                            optional: true,
                            placeholder: None,
                        },
                    ))
                    .field(Field::new(
                        "groups",
                        "Groups",
                        Kind::Chips {
                            options: Opt::names(["kelp", "reef"]),
                            free: false,
                        },
                    ))
                    .field(Field::new(
                        "files.answers",
                        "Answers from",
                        Kind::Choice {
                            options: vec![Opt::new("open", "Open"), Opt::new("files", "Files")],
                            none: None,
                        },
                    )),
                Section::new("Safety")
                    .field(Field::toggle("safety.crisis", "Crisis check"))
                    .field(Field::toggle("safety.breaks", "Breaks")),
            ],
        }
    }

    fn changes(v: Json) -> Map<String, Json> {
        v.as_object().unwrap().clone()
    }

    const FILE: &str = "\
# Mara — the owner's header comment.
display = \"Mara\"  # how she is shown

# who she spends time with
groups = [\"kelp\"]

[safety]
# on by default; the owner said so
crisis = true # keep this one
";

    /// The whole point: the owner's comments — above a key, trailing the
    /// very value changed, and the header — survive an edit.
    #[test]
    fn an_edit_changes_the_value_and_keeps_every_comment() {
        let out = apply(
            &form(),
            FILE,
            &changes(json!({ "safety.crisis": false, "display": "Mara K" })),
        )
        .unwrap();
        assert_eq!(
            out,
            FILE.replace("crisis = true", "crisis = false")
                .replace("\"Mara\"", "\"Mara K\"")
        );
    }

    #[test]
    fn nothing_changed_is_the_same_bytes() {
        assert_eq!(apply(&form(), FILE, &Map::new()).unwrap(), FILE);
        assert_eq!(
            apply(&form(), FILE, &changes(json!({ "groups": ["kelp"] }))).unwrap(),
            FILE
        );
    }

    /// A table the file does not have yet is made, as a table of its own.
    #[test]
    fn a_missing_table_is_made_and_a_null_removes_a_key() {
        let out = apply(
            &form(),
            FILE,
            &changes(json!({ "files.answers": "files", "display": "" })),
        )
        .unwrap();
        let back: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(back["files"]["answers"].as_str(), Some("files"));
        assert!(back.get("display").is_none(), "{out}");
        assert!(out.contains("# on by default; the owner said so"));
        assert!(out.contains("[files]"), "{out}");
    }

    /// The form is the fence: nothing outside it, nothing of the wrong kind.
    #[test]
    fn a_path_or_value_the_form_does_not_offer_is_refused_whole() {
        let refused = [
            json!({ "model": "anything" }),
            json!({ "safety.crisis": "no" }),
            json!({ "files.answers": "everything" }),
            json!({ "groups": ["kelp", "strangers"] }),
            json!({ "display": "two\nlines" }),
            json!({ "safety.crisis": null }),
            // One good change beside a bad one changes nothing.
            json!({ "safety.breaks": true, "tools.allow": ["shell"] }),
        ];
        for c in refused {
            assert!(apply(&form(), FILE, &changes(c.clone())).is_err(), "{c}");
        }
    }

    #[test]
    fn chips_are_trimmed_and_deduplicated() {
        let out = apply(
            &form(),
            FILE,
            &changes(json!({ "groups": [" reef", "kelp", "reef"] })),
        )
        .unwrap();
        assert!(out.contains("groups = [\"reef\", \"kelp\"]"), "{out}");
    }

    /// Values come from the typed document, so a default shows as itself.
    #[test]
    fn values_read_through_the_type() {
        let typed = json!({ "display": "Mara", "safety": { "crisis": true, "breaks": false } });
        let v = values(&form(), &typed);
        assert_eq!(v["safety.breaks"], json!(false));
        assert_eq!(v["display"], json!("Mara"));
        assert_eq!(v["files.answers"], Json::Null);
    }

    #[test]
    fn a_file_that_is_not_toml_is_refused_not_rewritten() {
        let err = apply(&form(), "display = ", &changes(json!({ "display": "x" })))
            .unwrap_err()
            .to_string();
        assert!(err.contains("edit it as text"), "{err}");
    }
}
