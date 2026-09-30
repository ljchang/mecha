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
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

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
    /// Text: one line, or with `multiline` several (newlines and tabs, no
    /// other control characters). With `optional`, an empty value removes
    /// the key.
    Text {
        max: usize,
        optional: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
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
    /// A list of tables (`[[line]]`), one row per table, matched by the
    /// `key` field, so a row that is edited keeps its comments and a row
    /// that moves carries them with it. `fields` are paths within a row
    /// (`text`, `sensor.kind`); a row's sub-table sent as `null` is removed.
    /// Sent whole, in order: the list is what the file will hold.
    Rows {
        key: String,
        fields: Vec<Field>,
        max_rows: usize,
    },
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
            (
                Kind::Text {
                    max,
                    optional,
                    multiline,
                    ..
                },
                Json::String(s),
            ) => {
                let s = s.trim();
                if s.is_empty() && *optional {
                    return Ok(Json::Null);
                }
                if s.chars().count() > *max {
                    bail!("`{path}` is at most {max} characters");
                }
                let allowed = |c: char| *multiline && (c == '\n' || c == '\t');
                if s.chars().any(|c| c.is_control() && !allowed(c)) {
                    bail!(if *multiline {
                        format!("`{path}` holds a control character")
                    } else {
                        format!("`{path}` is one line, with no control characters")
                    });
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
            (
                Kind::Rows {
                    key,
                    fields,
                    max_rows,
                },
                Json::Array(rows),
            ) => {
                if rows.len() > *max_rows {
                    bail!("`{path}` holds at most {max_rows} rows");
                }
                // A row may only carry what `fields` declares: the fence,
                // one level down.
                let tops: Vec<&str> = fields
                    .iter()
                    .map(|f| f.path.split('.').next().unwrap_or(""))
                    .collect();
                let mut seen: Vec<String> = Vec::new();
                let mut out = Vec::with_capacity(rows.len());
                for row in rows {
                    let Some(obj) = row.as_object() else {
                        bail!("each row of `{path}` is an object");
                    };
                    if let Some(extra) = obj.keys().find(|k| !tops.contains(&k.as_str())) {
                        bail!("`{extra}` is not a field of a `{path}` row");
                    }
                    let mut clean = Map::new();
                    for f in fields {
                        let (parent, leaf) = match f.path.split_once('.') {
                            Some((p, l)) => (Some(p), l),
                            None => (None, f.path.as_str()),
                        };
                        let got = match parent {
                            None => obj.get(leaf).cloned().unwrap_or(Json::Null),
                            Some(p) => match obj.get(p) {
                                // The whole sub-table absent: nothing to check.
                                None | Some(Json::Null) => {
                                    clean.insert(p.to_string(), Json::Null);
                                    continue;
                                }
                                Some(sub) => sub.get(leaf).cloned().unwrap_or(Json::Null),
                            },
                        };
                        let checked = Field {
                            path: format!("{path}[].{}", f.path),
                            ..f.clone()
                        }
                        .check(&got)?;
                        match parent {
                            None => {
                                clean.insert(leaf.to_string(), checked);
                            }
                            Some(p) => {
                                let sub = clean
                                    .entry(p.to_string())
                                    .or_insert_with(|| Json::Object(Map::new()));
                                if let Some(sub) = sub.as_object_mut() {
                                    sub.insert(leaf.to_string(), checked);
                                }
                            }
                        }
                    }
                    let id = clean
                        .get(key)
                        .and_then(Json::as_str)
                        .filter(|s| !s.is_empty())
                        .with_context(|| format!("each row of `{path}` needs a `{key}`"))?
                        .to_string();
                    if seen.contains(&id) {
                        bail!("two rows of `{path}` have the {key} `{id}`");
                    }
                    seen.push(id);
                    out.push(Json::Object(clean));
                }
                Ok(Json::Array(out))
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
        if let Some(Field {
            kind: Kind::Rows { key, .. },
            ..
        }) = form.field(path)
        {
            set_rows(
                &mut doc,
                path,
                key,
                v.as_array().map(Vec::as_slice).unwrap_or(&[]),
            )?;
        } else if v.is_null() {
            remove(&mut doc, path)?;
        } else {
            set(doc.as_table_mut(), path, &v)?;
        }
    }
    Ok(doc.to_string())
}

/// The comment lines anywhere in `t` — above its header, above its keys,
/// trailing its values, and in its sub-tables — for a table that is going,
/// so its comments are handed on rather than deleted.
fn comments_of(t: &Table) -> String {
    fn take(out: &mut String, raw: Option<&toml_edit::RawString>) {
        for line in raw.and_then(|r| r.as_str()).unwrap_or("").lines() {
            if let Some(at) = line.find('#') {
                out.push_str(line[at..].trim_end());
                out.push('\n');
            }
        }
    }
    let mut out = String::new();
    take(&mut out, t.decor().prefix());
    for (k, item) in t.iter() {
        if let Some((key, _)) = t.get_key_value(k) {
            take(&mut out, key.leaf_decor().prefix());
        }
        match item {
            Item::Value(v) => take(&mut out, v.decor().suffix()),
            Item::Table(sub) => out.push_str(&comments_of(sub)),
            _ => {}
        }
    }
    out
}

/// Does the file's item already say `v`? Compared as text, so an owner's
/// `setpoint = 3` is the form's `"3"` and a `'''literal'''` string is its
/// value: an unchanged field is left exactly as the owner wrote it, rather
/// than re-quoted on every save.
fn same_value(item: Option<&Item>, v: &Json) -> bool {
    let Some(item) = item.and_then(Item::as_value) else {
        return false;
    };
    let have = match item {
        Value::String(s) => s.value().to_string(),
        Value::Integer(i) => i.value().to_string(),
        Value::Float(f) => f.value().to_string(),
        Value::Boolean(b) => b.value().to_string(),
        _ => return false,
    };
    match v {
        Json::String(s) => *s == have,
        Json::Bool(b) => b.to_string() == have,
        Json::Number(n) => n.to_string() == have,
        _ => false,
    }
}

/// Set a list of tables to `rows`, in their order. A row whose key the file
/// already holds is edited in place; a table whose key is gone is removed
/// and its comments handed to the next table kept, or the end of the file;
/// a new key is a new table. Then every table and sub-table is renumbered,
/// since `toml_edit` writes tables in the order of their parsed positions
/// and would otherwise put a moved row back where it was.
fn set_rows(doc: &mut DocumentMut, path: &str, key: &str, rows: &[Json]) -> Result<()> {
    let wanted: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.get(key).and_then(Json::as_str))
        .collect();
    // A file of comments only (a charter's template) holds them all as the
    // document's trailing text, which is written *after* every table: taken
    // here, it goes above the first row instead.
    let root_empty = doc.as_table().is_empty();
    let root = doc.as_table_mut();
    let fresh_header = if root.get(path).is_none() {
        if rows.is_empty() {
            return Ok(());
        }
        root.insert(path, Item::ArrayOfTables(ArrayOfTables::new()));
        root_empty
    } else {
        false
    };
    let aot = root
        .get_mut(path)
        .and_then(Item::as_array_of_tables_mut)
        .with_context(|| format!("`{path}` is not a list of tables in the file"))?;
    let base = aot.iter().filter_map(Table::position).min().unwrap_or(1);
    let mut old = Vec::with_capacity(aot.len());
    while !aot.is_empty() {
        old.push(aot.remove(0));
    }
    // With no values above the list, `toml_edit` keeps the file's header on
    // the first table: it belongs to the list, not to that row, so it stays
    // at the top whichever row comes first now.
    let header = old.first_mut().map(|t| {
        let h = t
            .decor()
            .prefix()
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        t.decor_mut().set_prefix("\n");
        h
    });
    let mut carried = String::new();
    let mut kept: Vec<(String, Table)> = Vec::new();
    for mut t in old {
        let k = t.get(key).and_then(Item::as_str).map(str::to_string);
        match k {
            Some(k) if wanted.contains(&k.as_str()) && !kept.iter().any(|(x, _)| *x == k) => {
                if !carried.is_empty() {
                    let prefix = t
                        .decor()
                        .prefix()
                        .and_then(|r| r.as_str())
                        .unwrap_or("")
                        .to_string();
                    t.decor_mut().set_prefix(format!("{carried}{prefix}"));
                    carried.clear();
                }
                kept.push((k, t));
            }
            _ => carried.push_str(&comments_of(&t)),
        }
    }
    for row in rows {
        let Some(k) = row.get(key).and_then(Json::as_str) else {
            continue;
        };
        let mut t = match kept.iter().position(|(x, _)| x == k) {
            Some(at) => kept.remove(at).1,
            None => Table::new(),
        };
        let Some(obj) = row.as_object() else { continue };
        for (name, v) in obj {
            match v {
                Json::Null => {
                    // A sub-table sent as null goes, its comments handed on.
                    if let Some(Item::Table(sub)) = t.get(name) {
                        carried.push_str(&comments_of(sub));
                    }
                    t.remove(name);
                }
                Json::Object(sub) => {
                    for (leaf, v) in sub {
                        if v.is_null() {
                            if let Some(Item::Table(st)) = t.get_mut(name) {
                                st.remove(leaf);
                            }
                        } else if !same_value(t.get(name).and_then(|st| st.get(leaf)), v) {
                            set(&mut t, &format!("{name}.{leaf}"), v)?;
                        }
                    }
                }
                v if same_value(t.get(name), v) => {}
                v => set(&mut t, name, v)?,
            }
        }
        aot.push(t);
    }
    let header = header.or_else(|| fresh_header.then(String::new));
    if let (Some(header), Some(first)) = (header, aot.get_mut(0)) {
        let own = first
            .decor()
            .prefix()
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        let own = own.trim_start_matches('\n');
        let joined = if own.is_empty() {
            header
        } else {
            format!("{}\n\n{own}", header.trim_end())
        };
        first.decor_mut().set_prefix(joined);
    }
    let mut at = base;
    for t in aot.iter_mut() {
        t.set_position(Some(at));
        at += 1;
        for (_, item) in t.iter_mut() {
            if let Some(sub) = item.as_table_mut() {
                sub.set_position(Some(at));
                at += 1;
            }
        }
    }
    if fresh_header {
        let top = doc.trailing().as_str().unwrap_or("").to_string();
        doc.set_trailing("");
        if let Some(first) = doc
            .as_table_mut()
            .get_mut(path)
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|a| a.get_mut(0))
        {
            let own = first
                .decor()
                .prefix()
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            let top = top.trim_end();
            let joined = if top.is_empty() {
                own
            } else {
                format!("{top}\n\n{}", own.trim_start_matches('\n'))
            };
            first.decor_mut().set_prefix(joined);
        }
    }
    if !carried.is_empty() {
        let end = doc.trailing().as_str().unwrap_or("").to_string();
        doc.set_trailing(format!("{end}{carried}"));
    }
    Ok(())
}

/// Remove a key, keeping the owner's comments on it. `toml_edit` holds a
/// key's leading comment lines — the file's header, for the first key — on
/// the key itself, so a bare remove deletes them (review of #430). They are
/// handed to what comes next: the next key in the table, else (at the root)
/// the first table's header, else the end of the file. A comment trailing
/// the removed value goes with them.
fn remove(doc: &mut DocumentMut, path: &str) -> Result<()> {
    let segs: Vec<&str> = path.split('.').collect();
    let (leaf, parents) = segs.split_last().context("an empty path")?;
    let (carried, next) = {
        let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
        for seg in parents {
            match table.get_mut(seg).and_then(Item::as_table_like_mut) {
                Some(t) => table = t,
                // Removing under a table that is not there: done.
                None => return Ok(()),
            }
        }
        let Some((key, item)) = table.get_key_value(leaf) else {
            return Ok(());
        };
        let mut carried = key
            .leaf_decor()
            .prefix()
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        let trailing = item
            .as_value()
            .and_then(|v| v.decor().suffix())
            .and_then(|r| r.as_str())
            .and_then(|t| t.find('#').map(|at| t[at..].trim_end().to_string()));
        if let Some(t) = trailing {
            carried.push_str(&t);
            carried.push('\n');
        }
        // The next key the file shows after this one: a value, since a
        // sub-table renders after every value regardless of insertion order.
        let next = table
            .iter()
            .filter(|(_, i)| i.is_value())
            .map(|(k, _)| k.to_string())
            .skip_while(|k| k != leaf)
            .nth(1);
        table.remove(leaf);
        (carried, next)
    };
    if !carried.contains('#') {
        return Ok(());
    }
    let prepend = |decor: &mut toml_edit::Decor| {
        let old = decor
            .prefix()
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        decor.set_prefix(format!("{carried}{old}"));
    };
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for seg in parents {
        table = table
            .get_mut(seg)
            .and_then(Item::as_table_like_mut)
            .context("the table just edited")?;
    }
    if let Some(next) = next {
        if let Some(mut key) = table.key_mut(&next) {
            prepend(key.leaf_decor_mut());
            return Ok(());
        }
    }
    if parents.is_empty() {
        let first = doc
            .as_table_mut()
            .iter_mut()
            .filter_map(|(_, i)| i.as_table_mut())
            .min_by_key(|t| t.position().unwrap_or(isize::MAX));
        if let Some(t) = first {
            prepend(t.decor_mut());
            return Ok(());
        }
    }
    let end = doc.trailing().as_str().unwrap_or("").to_string();
    doc.set_trailing(format!("{end}{carried}"));
    Ok(())
}

fn set(root: &mut Table, path: &str, v: &Json) -> Result<()> {
    let segs: Vec<&str> = path.split('.').collect();
    let (leaf, parents) = segs.split_last().context("an empty path")?;
    let mut table: &mut dyn toml_edit::TableLike = root;
    for seg in parents {
        if table.get(seg).is_none() {
            table.insert(seg, Item::Table(Table::new()));
        }
        table = table
            .get_mut(seg)
            .and_then(Item::as_table_like_mut)
            .with_context(|| format!("`{seg}` in `{path}` is not a table in the file"))?;
    }
    // `null` never reaches here: `apply` sends it to `remove`.
    // A list already in the file is edited in place, so a comment inside it
    // stays (review of #430: a fresh array dropped them).
    if let (Json::Array(want), Some(Value::Array(have))) =
        (v, table.get_mut(leaf).and_then(Item::as_value_mut))
    {
        edit_array(have, want);
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

/// `have` edited to hold `want`'s names: entries no longer wanted removed —
/// each handing its leading comments on, since `toml_edit` keeps a comment
/// that trails an entry on the *next* one's decor — and new ones appended,
/// indented like their neighbours in a multi-line list.
fn edit_array(have: &mut Array, want: &[Json]) {
    let want: Vec<&str> = want.iter().filter_map(Json::as_str).collect();
    let prefix_of = |v: &Value| {
        v.decor()
            .prefix()
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string()
    };
    let mut i = have.len();
    while i > 0 {
        i -= 1;
        let keep = have
            .get(i)
            .and_then(Value::as_str)
            .is_some_and(|s| want.contains(&s));
        if keep {
            continue;
        }
        let carried = have.get(i).map(prefix_of).unwrap_or_default();
        have.remove(i);
        if carried.contains('#') {
            match have.get_mut(i) {
                Some(next) => {
                    let old = prefix_of(next);
                    next.decor_mut().set_prefix(format!("{carried}{old}"));
                }
                None => {
                    let end = have.trailing().as_str().unwrap_or("").to_string();
                    have.set_trailing(format!("{carried}{end}"));
                }
            }
        }
    }
    // The indentation a multi-line list uses: the whitespace after the last
    // newline in an entry's prefix.
    let indent = have.iter().last().map(prefix_of).and_then(|p| {
        p.rfind('\n')
            .map(|at| &p[at..])
            .filter(|tail| tail.trim().is_empty())
            .map(str::to_string)
    });
    for &name in &want {
        if have.iter().any(|x| x.as_str() == Some(name)) {
            continue;
        }
        let mut v = Value::from(name);
        if let Some(indent) = &indent {
            v.decor_mut().set_prefix(indent.clone());
        }
        have.push_formatted(v);
    }
    // In the order asked for, each entry moving with its own decor.
    let order: Vec<Option<String>> = have
        .iter()
        .map(|x| x.as_str().map(str::to_string))
        .collect();
    if order
        .iter()
        .map(|o| o.as_deref())
        .ne(want.iter().map(|w| Some(*w)))
    {
        let mut taken: Vec<Value> = Vec::with_capacity(have.len());
        while !have.is_empty() {
            taken.push(have.remove(0));
        }
        for &name in &want {
            if let Some(at) = taken.iter().position(|x| x.as_str() == Some(name)) {
                have.push_formatted(taken.remove(at));
            }
        }
    }
    // A one-line list keeps one-line spacing: `["a", "b"]`, whatever moved.
    if indent.is_none() {
        for (i, v) in have.iter_mut().enumerate() {
            let plain = v
                .decor()
                .prefix()
                .and_then(|r| r.as_str())
                .is_none_or(|p| !p.contains(['#', '\n']));
            if plain {
                v.decor_mut().set_prefix(if i == 0 { "" } else { " " });
            }
        }
    }
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
                            multiline: false,
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

    fn comment_lines(t: &str) -> Vec<String> {
        let mut out: Vec<String> = t
            .lines()
            .filter_map(|l| l.find('#').map(|at| l[at..].trim().to_string()))
            .collect();
        out.sort();
        out
    }

    /// Clearing a field removes its key and never the owner's comments on
    /// it — here the file's header, which `toml_edit` keeps on the first key,
    /// and the comment trailing the value (review of #430).
    #[test]
    fn a_removed_key_leaves_every_comment_behind() {
        let out = apply(&form(), FILE, &changes(json!({ "display": "" }))).unwrap();
        assert!(!out.contains("display"), "{out}");
        assert_eq!(comment_lines(&out), comment_lines(FILE), "{out}");
        assert!(
            out.starts_with("# Mara — the owner's header comment."),
            "{out}"
        );

        // The last key before a table: onto the table's header. The last key
        // of a table: onto the end of the file. Each still loads.
        let text_form = |path: &str| Form {
            sections: vec![Section::new("s").field(Field::new(
                path,
                "x",
                Kind::Text {
                    max: 9,
                    optional: true,
                    placeholder: None,
                    multiline: false,
                },
            ))],
        };
        let file = "a = 1\n# about b\nb = \"y\" # trailing\n\n[t]\n# about c\nc = \"z\"\n";
        for path in ["b", "t.c"] {
            let out = apply(&text_form(path), file, &changes(json!({ path: null }))).unwrap();
            assert_eq!(comment_lines(&out), comment_lines(file), "{path}: {out}");
            let back: toml::Table = toml::from_str(&out).unwrap();
            assert_eq!(back["a"].as_integer(), Some(1));
        }
    }

    /// A list edited from the form keeps the comments written inside it,
    /// on the entries that stay and on the ones removed around them.
    #[test]
    fn a_multi_line_list_keeps_its_comments() {
        let chips = Form {
            sections: vec![Section::new("s").field(Field::new(
                "tools.allow",
                "Tools",
                Kind::Chips {
                    options: Vec::new(),
                    free: true,
                },
            ))],
        };
        let file =
            "[tools]\nallow = [\n  \"web_search\",   # she looks things up\n  \"image_view\",\n]\n";
        let out = apply(
            &chips,
            file,
            &changes(json!({ "tools.allow": ["web_search", "fs_read"] })),
        )
        .unwrap();
        assert!(out.contains("# she looks things up"), "{out}");
        let back: toml::Table = toml::from_str(&out).unwrap();
        let names: Vec<&str> = back["tools"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(names, ["web_search", "fs_read"], "{out}");
        // The commented entry itself removed: its comment is handed on.
        let out = apply(
            &chips,
            file,
            &changes(json!({ "tools.allow": ["image_view"] })),
        )
        .unwrap();
        assert_eq!(comment_lines(&out), comment_lines(file), "{out}");
        assert!(toml::from_str::<toml::Table>(&out).is_ok(), "{out}");
    }

    fn rows_form() -> Form {
        let text = |max, multiline| Kind::Text {
            max,
            optional: false,
            placeholder: None,
            multiline,
        };
        Form {
            sections: vec![Section::new("Lines").field(Field::new(
                "line",
                "Lines",
                Kind::Rows {
                    key: "id".into(),
                    fields: vec![
                        Field::new("id", "Id", text(60, false)),
                        Field::new("text", "Text", text(2000, true)),
                        Field::new(
                            "sensor.kind",
                            "Sensor",
                            Kind::Choice {
                                options: vec![Opt::new("outbox_waiting", "Outbox waiting")],
                                none: None,
                            },
                        ),
                        Field::new("sensor.setpoint", "Setpoint", text(40, false)),
                    ],
                    max_rows: 3,
                },
            ))],
        }
    }

    const CHARTER: &str = "\
# The owner's charter.
# Order is rank.

[[line]]
# why this one matters
id = \"ship\"
text = \"Ship the paper\"  # the big one

[[line]]
id = \"sleep\"
text = \"Sleep eight hours\"

[line.sensor]
kind = \"outbox_waiting\"
setpoint = 3
";

    fn lines(out: &str) -> Vec<String> {
        let t: toml::Table = toml::from_str(out).expect(out);
        t["line"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["id"].as_str().unwrap().to_string())
            .collect()
    }

    fn rows(v: Json) -> Map<String, Json> {
        changes(json!({ "line": v }))
    }

    /// A list of tables edited from the form: rows matched by id, comments
    /// kept on the rows that stay and handed on from the rows that go, the
    /// order the page sent, and an unchanged value left as the owner wrote it.
    #[test]
    fn rows_are_edited_in_place_reordered_added_and_removed() {
        let ship = json!({"id": "ship", "text": "Ship the paper", "sensor": null});
        let sleep = json!({"id": "sleep", "text": "Sleep eight hours",
                           "sensor": {"kind": "outbox_waiting", "setpoint": "3"}});
        // The same list is the same bytes: `setpoint = 3` stays a number.
        assert_eq!(
            apply(&rows_form(), CHARTER, &rows(json!([ship, sleep]))).unwrap(),
            CHARTER
        );

        // Reordered: the rows swap, each with its own comments.
        let out = apply(&rows_form(), CHARTER, &rows(json!([sleep, ship]))).unwrap();
        assert_eq!(lines(&out), ["sleep", "ship"], "{out}");
        assert_eq!(comment_lines(&out), comment_lines(CHARTER), "{out}");
        assert!(out.starts_with("# The owner's charter."), "{out}");
        assert!(out.contains("setpoint = 3"), "{out}");
        let why = out.find("# why this one matters").unwrap();
        assert!(
            why > out.find("id = \"sleep\"").unwrap(),
            "the comment moved with ship: {out}"
        );

        // Edited: the text changes, its trailing comment stays.
        let edited = json!({"id": "ship", "text": "Ship the paper by Friday", "sensor": null});
        let out = apply(&rows_form(), CHARTER, &rows(json!([edited, sleep]))).unwrap();
        assert!(
            out.contains("text = \"Ship the paper by Friday\"  # the big one"),
            "{out}"
        );

        // Removed and added: ship's comments are handed on, walk is new, and
        // sleep's sensor goes with its sub-table.
        let walk = json!({"id": "walk", "text": "Walk\ntwice", "sensor": null});
        let bare = json!({"id": "sleep", "text": "Sleep eight hours", "sensor": null});
        let out = apply(&rows_form(), CHARTER, &rows(json!([bare, walk]))).unwrap();
        assert_eq!(lines(&out), ["sleep", "walk"], "{out}");
        assert_eq!(comment_lines(&out), comment_lines(CHARTER), "{out}");
        assert!(!out.contains("[line.sensor]"), "{out}");
        let back: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(back["line"][1]["text"].as_str(), Some("Walk\ntwice"));

        // A changed setpoint is written; a new sensor is a new sub-table.
        let sensed = json!({"id": "ship", "text": "Ship the paper",
                            "sensor": {"kind": "outbox_waiting", "setpoint": "5"}});
        let out = apply(&rows_form(), CHARTER, &rows(json!([sensed]))).unwrap();
        let back: toml::Table = toml::from_str(&out).expect(&out);
        assert_eq!(
            back["line"][0]["sensor"]["setpoint"].as_str(),
            Some("5"),
            "{out}"
        );

        // From no list at all: the header stays, the rows follow it.
        let header = "# Only comments so far.\n";
        let out = apply(&rows_form(), header, &rows(json!([walk]))).unwrap();
        assert!(out.starts_with(header), "{out}");
        assert_eq!(lines(&out), ["walk"]);
    }

    #[test]
    fn a_row_the_form_does_not_offer_is_refused_whole() {
        let ok = json!({"id": "a", "text": "x", "sensor": null});
        for bad in [
            json!([{"id": "a", "text": "x", "rank": 1}]),
            json!([ok, {"id": "a", "text": "y", "sensor": null}]),
            json!([{"id": "", "text": "x", "sensor": null}]),
            json!([{"id": "a", "text": "x", "sensor": {"kind": "made_up", "setpoint": "1"}}]),
            json!([{"id": "a b\u{1b}", "text": "x", "sensor": null}]),
            json!([ok, {"id": "b", "text": "x"}, {"id": "c", "text": "x"}, {"id": "d", "text": "x"}]),
            json!({"id": "a"}),
        ] {
            assert!(
                apply(&rows_form(), CHARTER, &rows(bad.clone())).is_err(),
                "{bad}"
            );
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
