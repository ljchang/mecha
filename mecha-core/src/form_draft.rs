//! A form's draft: what the owner edits on the Factory page before
//! publishing it, kept apart from what is served.
//!
//! **The served copy is never the edit target.** `~/.mecha/factory/types/
//! <id>.toml` is the factory's record of what the box serves, and the drain
//! validates strangers' submissions against it; a half-made field there would
//! invalidate real requests. So the owner edits `~/.mecha/factory/drafts/
//! <id>.toml`, and **Publish** is `factory-publish type push` of the draft,
//! which rewrites the served copy only once the box has accepted it.
//!
//! **Hours stay the hours editor's.** A booking page's draft carries an
//! `[availability]` too, but publishing it takes the served copy's hours
//! (all but `timezone`, which only a publish can change) over the draft's —
//! so a draft started before an hours save can never put old hours back
//! ([`carry_hours`]).
//!
//! **The owner is the only author**, as for hours: the writers here are
//! reached only from the web handler (`only_the_owners_page_writes_forms`).
//!
//! **A write keeps the file.** `toml_edit`, matching fields by `name`, so a
//! field's keys the editor does not show (`show_when`, `pattern`, `min`) and
//! every comment survive; the factory's `type check` judges the result.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table};

/// `~/.mecha/factory/drafts`.
pub fn drafts_dir() -> Result<PathBuf> {
    Ok(crate::work::mecha_home()?.join("factory").join("drafts"))
}

/// A form id as the factory accepts one: it names a file here and a URL
/// there, so nothing else is joined onto a path.
pub fn valid_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && id.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    if !ok {
        bail!(
            "`{id}` is not a form id: lowercase letters, digits, - and _, starting with a letter"
        );
    }
    Ok(())
}

pub fn draft_path(dir: &Path, id: &str) -> Result<PathBuf> {
    valid_id(id)?;
    Ok(dir.join(format!("{id}.toml")))
}

/// The kinds the editor offers for a new field. `file` is kept on fields
/// that have it but not offered: its size and type allowances are a
/// decision the TOML states better than a picker.
pub const KINDS: [&str; 9] = [
    "text",
    "long_text",
    "email",
    "url",
    "date",
    "integer",
    "select",
    "multi_select",
    "bool",
];

/// Keys that belong to a kind, so a kind change can drop the old ones — the
/// factory refuses a field carrying a key its kind does not take.
fn kind_keys(kind: &str) -> &'static [&'static str] {
    match kind {
        "text" => &["max_length", "pattern"],
        "long_text" | "email" | "url" => &["max_length"],
        "date" | "integer" => &["min", "max"],
        "select" => &["options"],
        "multi_select" => &["options", "max_choices"],
        "file" => &["max_bytes", "accept"],
        _ => &[],
    }
}
const ALL_KIND_KEYS: [&str; 8] = [
    "max_length",
    "pattern",
    "min",
    "max",
    "options",
    "max_choices",
    "max_bytes",
    "accept",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

/// One field as the editor shows it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FieldEdit {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub help: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    /// For `select` and `multi_select`.
    #[serde(default)]
    pub options: Option<Vec<Choice>>,
    /// For the text kinds; the factory requires it on `text` and `long_text`.
    #[serde(default)]
    pub max_length: Option<u64>,
}

/// What the structured editor owns of a manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FormEdit {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub confirmation_heading: Option<String>,
    #[serde(default)]
    pub confirmation_body: Option<String>,
    pub fields: Vec<FieldEdit>,
}

fn s(item: Option<&Item>) -> Option<String> {
    item.and_then(Item::as_str).map(str::to_string)
}

/// Read the structured part of a manifest.
pub fn read_form(text: &str) -> Result<FormEdit> {
    let doc: DocumentMut = text.parse().context("parsing the manifest")?;
    let title = s(doc.get("title")).context("the manifest has no title")?;
    let confirmation = doc.get("confirmation").and_then(Item::as_table);
    let mut fields = Vec::new();
    if let Some(list) = doc.get("fields").and_then(Item::as_array_of_tables) {
        for t in list.iter() {
            let get = |k: &str| t.get(k);
            let options = get("options").and_then(Item::as_array).map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_inline_table())
                    .map(|c| Choice {
                        value: c
                            .get("value")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        label: c
                            .get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            });
            fields.push(FieldEdit {
                name: s(get("name")).context("a field has no name")?,
                label: s(get("label")).unwrap_or_default(),
                help: s(get("help")),
                kind: s(get("kind")).unwrap_or_else(|| "text".into()),
                required: get("required").and_then(Item::as_bool).unwrap_or(false),
                options,
                max_length: get("max_length")
                    .and_then(Item::as_integer)
                    .and_then(|n| u64::try_from(n).ok()),
            });
        }
    }
    Ok(FormEdit {
        title,
        description: s(doc.get("description")),
        confirmation_heading: confirmation.and_then(|c| s(c.get("heading"))),
        confirmation_body: confirmation.and_then(|c| s(c.get("body"))),
        fields,
    })
}

/// What would make a write unreadable or certain to be refused, in the
/// editor's terms; the factory's `type check` judges the rest.
pub fn validate(f: &FormEdit) -> Result<()> {
    if f.title.trim().is_empty() {
        bail!("give the form a title");
    }
    if f.fields.is_empty() {
        bail!("a form needs at least one field");
    }
    if f.fields.len() > 64 {
        bail!("at most 64 fields");
    }
    let mut seen = std::collections::BTreeSet::new();
    for field in &f.fields {
        let ok = field
            .name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
            && field
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ok {
            bail!(
                "`{}` is not a field name: lowercase letters, digits and _",
                field.name
            );
        }
        if !seen.insert(field.name.as_str()) {
            bail!("two fields are named `{}`", field.name);
        }
        if field.label.trim().is_empty() {
            bail!("the `{}` field needs a label", field.name);
        }
        if !KINDS.contains(&field.kind.as_str()) && field.kind != "file" {
            bail!("`{}` is not a field kind", field.kind);
        }
        if matches!(field.kind.as_str(), "select" | "multi_select")
            && field.options.as_ref().is_none_or(|o| o.is_empty())
        {
            bail!(
                "`{}` is a choice, so it needs at least one option",
                field.label
            );
        }
    }
    Ok(())
}

/// `text` with its structured part replaced by `f`, and everything else kept.
pub fn write_form(text: &str, f: &FormEdit) -> Result<String> {
    validate(f)?;
    let mut doc: DocumentMut = text.parse().context("parsing the manifest")?;
    let set_str = |t: &mut Table, key: &str, v: &str| {
        let mut new = toml_edit::Value::from(v);
        if let Some(old) = t.get(key).and_then(Item::as_value) {
            *new.decor_mut() = old.decor().clone();
        }
        t[key] = Item::Value(new);
    };
    let root = doc.as_table_mut();
    set_str(root, "title", &f.title);
    match f.description.as_deref().filter(|d| !d.trim().is_empty()) {
        Some(d) => set_str(root, "description", d),
        None => {
            root.remove("description");
        }
    }
    if f.confirmation_heading.is_some() || f.confirmation_body.is_some() {
        if !root.contains_key("confirmation") {
            root["confirmation"] = Item::Table(Table::new());
        }
        let c = root["confirmation"]
            .as_table_mut()
            .context("[confirmation] is not a table")?;
        set_str(
            c,
            "heading",
            f.confirmation_heading.as_deref().unwrap_or(""),
        );
        set_str(c, "body", f.confirmation_body.as_deref().unwrap_or(""));
    }

    // Fields: matched by name, so an existing one keeps its unseen keys,
    // its comments and its place; a new one starts bare.
    let old: Vec<Table> = root
        .get("fields")
        .and_then(Item::as_array_of_tables)
        .map(|a| a.iter().cloned().collect())
        .unwrap_or_default();
    let at = old.first().and_then(Table::position);
    let mut fields = ArrayOfTables::new();
    for field in &f.fields {
        let mut t = old
            .iter()
            .find(|t| t.get("name").and_then(Item::as_str) == Some(field.name.as_str()))
            .cloned()
            .unwrap_or_default();
        let was = t.get("kind").and_then(Item::as_str).map(str::to_string);
        if was.as_deref() != Some(field.kind.as_str())
            && (was.as_deref() == Some("file") || field.kind == "file")
        {
            bail!(
                "`{}` cannot change to or from a file field here; edit it as TOML",
                field.label
            );
        }
        set_str(&mut t, "name", &field.name);
        set_str(&mut t, "label", &field.label);
        match field.help.as_deref().filter(|h| !h.trim().is_empty()) {
            Some(h) => set_str(&mut t, "help", h),
            None => {
                t.remove("help");
            }
        }
        set_str(&mut t, "kind", &field.kind);
        // A kind change drops the keys the old kind took and the new one
        // does not, or the factory refuses the field.
        let keep = kind_keys(&field.kind);
        for key in ALL_KIND_KEYS {
            if !keep.contains(&key) {
                t.remove(key);
            }
        }
        // `text` and `long_text` require a cap; `email` and `url` default
        // one, so theirs is written only when stated.
        let required_cap = matches!(field.kind.as_str(), "text" | "long_text");
        let optional_cap = matches!(field.kind.as_str(), "email" | "url");
        if required_cap || optional_cap {
            let stated = field.max_length.or_else(|| {
                t.get("max_length")
                    .and_then(Item::as_integer)
                    .map(|n| n as u64)
            });
            let default = if field.kind == "long_text" { 2000 } else { 200 };
            match (stated, required_cap) {
                (Some(n), _) => t["max_length"] = value(i64::try_from(n).unwrap_or(i64::MAX)),
                (None, true) => t["max_length"] = value(default),
                (None, false) => {}
            }
        }
        if let Some(options) = field.options.as_ref().filter(|_| keep.contains(&"options")) {
            let mut arr = Array::new();
            for c in options {
                let mut it = InlineTable::new();
                it.insert("value", c.value.as_str().into());
                it.insert("label", c.label.as_str().into());
                arr.push(toml_edit::Value::InlineTable(it));
            }
            t["options"] = value(arr);
        }
        if field.required {
            t["required"] = value(true);
        } else {
            t.remove("required");
        }
        t.set_position(at);
        fields.push(t);
    }
    root["fields"] = Item::ArrayOfTables(fields);
    Ok(doc.to_string())
}

/// A booking draft with the served copy's hours put back over its own,
/// all but `timezone`: the hours editor owns them, and a draft started
/// before an hours save must not undo it on publish. A form, or a served
/// copy with no hours, is returned unchanged.
pub fn carry_hours(draft: &str, served: &str) -> Result<String> {
    let mut doc: DocumentMut = draft.parse().context("parsing the draft")?;
    let served: DocumentMut = served.parse().context("parsing the served copy")?;
    let (Some(from), Some(to)) = (
        served.get("availability").and_then(Item::as_table),
        doc.get("availability").and_then(Item::as_table),
    ) else {
        return Ok(draft.to_string());
    };
    let zone = to.get("timezone").cloned();
    let at = to.position();
    let mut carried = from.clone();
    if let Some(zone) = zone {
        carried["timezone"] = zone;
    }
    carried.set_position(at);
    for key in ["windows", "overrides"] {
        if let Some(list) = carried.get_mut(key).and_then(Item::as_array_of_tables_mut) {
            for t in list.iter_mut() {
                t.set_position(at);
            }
        }
    }
    doc["availability"] = Item::Table(carried);
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORM: &str = r#"# The owner's note above everything.
id = "meeting"
version = 1
title = "Request a meeting"
description = "A few questions."

[[fields]]
name = "requester_email"
label = "Your email"
kind = "email"
required = true

# Why this one is a select.
[[fields]]
name = "purpose"
label = "What it is about"
kind = "select"
required = true
options = [
  { value = "advising", label = "Advising" },
]

[[fields]]
name = "detail"
label = "Say more"
kind = "long_text"
max_length = 600
show_when = { field = "purpose", equals = "advising" }

[verification]
field = "requester_email"

[confirmation]
heading = "Thanks"
body = "I'll write back."
expect_reply_within = "a week"
"#;

    #[test]
    fn a_form_reads_and_writes_back_unchanged_in_meaning() {
        let f = read_form(FORM).unwrap();
        assert_eq!(f.title, "Request a meeting");
        assert_eq!(f.fields.len(), 3);
        assert_eq!(f.fields[1].options.as_ref().unwrap()[0].value, "advising");
        let out = write_form(FORM, &f).unwrap();
        assert_eq!(read_form(&out).unwrap(), f);
        // What the editor does not show survives: comments, show_when, the
        // confirmation's promise, verification.
        assert!(out.starts_with("# The owner's note above everything."));
        assert!(out.contains("# Why this one is a select."), "{out}");
        assert!(out.contains("show_when"), "{out}");
        assert!(out.contains("expect_reply_within = \"a week\""), "{out}");
        assert!(out.contains("[verification]"), "{out}");
    }

    #[test]
    fn editing_reorders_adds_removes_and_changes_kinds_cleanly() {
        let mut f = read_form(FORM).unwrap();
        // Reorder, drop `detail`, add a new text field, turn purpose into text.
        let email = f.fields.remove(0);
        f.fields.retain(|x| x.name != "detail");
        f.fields.push(email);
        f.fields[0].kind = "text".into();
        f.fields[0].options = None;
        f.fields.push(FieldEdit {
            name: "affiliation".into(),
            label: "Where you're from".into(),
            help: Some("Department or company".into()),
            kind: "text".into(),
            required: false,
            options: None,
            max_length: None,
        });
        let out = write_form(FORM, &f).unwrap();
        let back = read_form(&out).unwrap();
        let names: Vec<_> = back.fields.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["purpose", "requester_email", "affiliation"]);
        // purpose became text: its options are gone, a max_length came.
        assert!(back.fields[0].options.is_none());
        assert_eq!(back.fields[0].max_length, Some(200));
        assert!(
            !out.contains("show_when"),
            "the removed field's keys went with it"
        );
        // The fields are still before [verification], not after the file's end.
        assert!(
            out.find("[[fields]]").unwrap() < out.find("[verification]").unwrap(),
            "{out}"
        );
    }

    #[test]
    fn what_the_factory_would_refuse_is_refused_first() {
        let refuse = |edit: fn(&mut FormEdit), says: &str| {
            let mut f = read_form(FORM).unwrap();
            edit(&mut f);
            let err = write_form(FORM, &f).unwrap_err().to_string();
            assert!(err.contains(says), "`{err}` should say `{says}`");
        };
        refuse(|f| f.title.clear(), "title");
        refuse(|f| f.fields.clear(), "at least one field");
        refuse(
            |f| f.fields[1].name = "requester_email".into(),
            "two fields",
        );
        refuse(
            |f| f.fields[0].name = "Your Email".into(),
            "not a field name",
        );
        refuse(
            |f| f.fields[1].options = Some(vec![]),
            "needs at least one option",
        );
        refuse(|f| f.fields[0].kind = "file".into(), "file field");
    }

    #[test]
    fn publishing_a_booking_draft_keeps_the_served_hours_and_the_drafts_zone() {
        let served = "id = \"book\"\nkind = \"booking\"\ntitle = \"T\"\n\n[availability]\ntimezone = \"America/New_York\"\ndurations = [30]\n\n[[availability.windows]]\nday = \"mon\"\nstart = \"09:00\"\nend = \"10:00\"\n\n[[fields]]\nname = \"a\"\nlabel = \"A\"\nkind = \"bool\"\n";
        let draft = served
            .replace("day = \"mon\"", "day = \"fri\"")
            .replace("America/New_York", "Europe/Helsinki")
            .replace("title = \"T\"", "title = \"New title\"");
        let out = carry_hours(&draft, served).unwrap();
        assert!(out.contains("day = \"mon\""), "the served hours win: {out}");
        assert!(!out.contains("day = \"fri\""), "{out}");
        assert!(
            out.contains("Europe/Helsinki"),
            "the draft's zone stays: {out}"
        );
        assert!(
            out.contains("New title"),
            "the rest of the draft stays: {out}"
        );
        // A form has no hours: unchanged.
        assert_eq!(carry_hours(FORM, FORM).unwrap(), FORM);
    }

    #[test]
    fn a_form_id_names_nothing_but_a_file_here() {
        assert!(valid_id("office-hours").is_ok());
        for bad in ["", "../x", "Book", "9am", "a/b", "a.toml"] {
            assert!(valid_id(bad).is_err(), "{bad}");
        }
    }

    /// "The owner is the only author" for forms, as for hours: nothing but
    /// this module and the web handler may reach a draft writer.
    #[test]
    fn only_the_owners_page_writes_forms() {
        const ALLOWED: [&str; 2] = [
            "mecha-core/src/form_draft.rs",
            "mecha-cli/src/commands/serve/factory.rs",
        ];
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut offenders = Vec::new();
        let mut stack: Vec<PathBuf> = [
            "mecha-core/src",
            "mecha-cli/src",
            "mecha-mail/src",
            "mecha-slack/src",
        ]
        .iter()
        .map(|d| root.join(d))
        .collect();
        let mut seen = 0;
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                seen += 1;
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if ALLOWED.contains(&rel.as_str()) {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                if text.contains("form_draft") {
                    offenders.push(rel);
                }
            }
        }
        assert!(seen > 100, "the walk read the tree ({seen} files)");
        // lib.rs declares the module; that is not a caller.
        offenders.retain(|f| f != "mecha-core/src/lib.rs");
        assert!(
            offenders.is_empty(),
            "the forms strangers fill in are the owner's to write, from the web page only; \
             these reach the draft module: {offenders:?}"
        );
    }
}
