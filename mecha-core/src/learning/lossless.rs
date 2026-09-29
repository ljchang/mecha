//! Rewriting a stored record without losing what this build cannot read.
//!
//! The learning store's rewrites go through typed structs, and a typed
//! round trip is not identity: a field a newer binary wrote is not in the
//! struct, so re-serialising drops it, and a line that does not parse at all
//! was never in the vector to be written back. Each rewrite here compares
//! three trees per record — what was on disk (`orig`), what this build makes
//! of it (`reparsed`, the parsed struct serialised again), and what the
//! caller wants written (`new`):
//!
//! - `new == reparsed` — the caller did not touch it — keeps `orig`
//!   byte for byte, lenient-degraded values included;
//! - a key in `orig` that `reparsed` lacks is one this build does not know,
//!   and is carried into the output unless `new` sets it;
//! - a key in both that `new` omits was cleared by the caller (an `Option`
//!   set to `None`), and stays cleared — which is why `reparsed` is needed
//!   at all: diffing `new` against `orig` alone cannot tell "cleared" from
//!   "unknown", and would resurrect every cleared field.
//!
//! Objects recurse, so an unknown key inside a nested table survives an edit
//! to its sibling. Arrays are leaves: a changed array is written as `new`.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

/// The JSON record to write back. See the module doc.
pub(super) fn merge_json(
    orig: &serde_json::Value,
    reparsed: &serde_json::Value,
    new: &serde_json::Value,
) -> serde_json::Value {
    if new == reparsed {
        return orig.clone();
    }
    let (Some(o), Some(r), Some(n)) = (orig.as_object(), reparsed.as_object(), new.as_object())
    else {
        return new.clone();
    };
    let mut out = n.clone();
    for (k, nv) in n {
        if let (Some(ov), Some(rv)) = (o.get(k), r.get(k)) {
            out.insert(k.clone(), merge_json(ov, rv, nv));
        }
    }
    for (k, ov) in o {
        if !r.contains_key(k) && !n.contains_key(k) {
            out.insert(k.clone(), ov.clone());
        }
    }
    serde_json::Value::Object(out)
}

/// [`merge_json`]'s twin over TOML, for the rules files.
pub(super) fn merge_toml(
    orig: &toml::Value,
    reparsed: &toml::Value,
    new: &toml::Value,
) -> toml::Value {
    if new == reparsed {
        return orig.clone();
    }
    let (Some(o), Some(r), Some(n)) = (orig.as_table(), reparsed.as_table(), new.as_table()) else {
        return new.clone();
    };
    let mut out = n.clone();
    for (k, nv) in n {
        if let (Some(ov), Some(rv)) = (o.get(k), r.get(k)) {
            out.insert(k.clone(), merge_toml(ov, rv, nv));
        }
    }
    for (k, ov) in o {
        if !r.contains_key(k) && !n.contains_key(k) {
            out.insert(k.clone(), ov.clone());
        }
    }
    toml::Value::Table(out)
}

/// One stored JSON record, rendered for a rewrite: the original text when the
/// caller left it alone, the struct's own rendering when nothing from the
/// original needs carrying (so the common case keeps the struct's field
/// order), and the merged tree only when something does — whose keys come
/// out sorted, the price of carrying a field this build has no slot for.
pub(super) fn render_json<T: serde::Serialize>(
    orig_text: &str,
    orig: &serde_json::Value,
    reparsed: &serde_json::Value,
    new: &T,
    pretty: bool,
) -> Result<Option<String>> {
    let new_tree = serde_json::to_value(new)?;
    if &new_tree == reparsed {
        return Ok(None);
    }
    let merged = merge_json(orig, reparsed, &new_tree);
    let text = match (merged == new_tree, pretty) {
        (true, true) => serde_json::to_string_pretty(new)?,
        (true, false) => serde_json::to_string(new)?,
        (false, true) => serde_json::to_string_pretty(&merged)?,
        (false, false) => serde_json::to_string(&merged)?,
    };
    // A merge can land back on the original bytes (the only change was to a
    // value this build had degraded); that is still "nothing to write".
    Ok((text != orig_text).then_some(text))
}

/// Temp sibling, fsync, the original's permissions, rename: every file here
/// is read without a lock by something (the rules at every run start), so it
/// must be whole at every instant, owner-only stays owner-only, and a crash
/// after the rename must not leave a renamed-but-empty file.
pub(super) fn write_replacing(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if let Ok(meta) = std::fs::metadata(path) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_untouched_record_keeps_its_original_tree() {
        let orig = json!({"a": 1, "surface": "from-the-future"});
        let reparsed = json!({"a": 1, "surface": "other"});
        assert_eq!(merge_json(&orig, &reparsed, &reparsed), orig);
    }

    #[test]
    fn an_unknown_key_survives_an_edit_to_its_sibling() {
        let orig = json!({"a": 1, "newer": "kept", "nested": {"b": 2, "deeper": true}});
        let reparsed = json!({"a": 1, "nested": {"b": 2}});
        let new = json!({"a": 5, "nested": {"b": 3}});
        assert_eq!(
            merge_json(&orig, &reparsed, &new),
            json!({"a": 5, "newer": "kept", "nested": {"b": 3, "deeper": true}})
        );
    }

    /// The case `reparsed` exists for: a field the caller cleared is known to
    /// this build, so it is not "unknown" and must not come back.
    #[test]
    fn a_cleared_field_stays_cleared() {
        let orig = json!({"a": 1, "dropped_at": "2026-09-01"});
        let reparsed = orig.clone();
        let new = json!({"a": 1});
        assert_eq!(merge_json(&orig, &reparsed, &new), json!({"a": 1}));
    }

    #[test]
    fn an_edited_value_wins_over_a_degraded_original() {
        let orig = json!({"surface": "from-the-future", "a": 1});
        let reparsed = json!({"surface": "other", "a": 1});
        let new = json!({"surface": "chat", "a": 1});
        assert_eq!(merge_json(&orig, &reparsed, &new), new);
    }

    #[test]
    fn the_toml_twin_carries_unknown_keys_too() {
        let orig: toml::Value =
            toml::from_str("text = 'a'\nnewer = 1\nretired_at = 'x'\n").unwrap();
        let reparsed: toml::Value = toml::from_str("text = 'a'\nretired_at = 'x'\n").unwrap();
        let new: toml::Value = toml::from_str("text = 'b'\n").unwrap();
        let want: toml::Value = toml::from_str("text = 'b'\nnewer = 1\n").unwrap();
        assert_eq!(merge_toml(&orig, &reparsed, &new), want);
    }

    #[test]
    fn replacing_keeps_the_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mecha-lossless-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.jsonl");
        std::fs::write(&path, "old\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_replacing(&path, b"new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!dir.join("f.jsonl.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// The three rewrites, against files holding what this build cannot read.
/// Each fails on the typed round trip these replaced.
#[cfg(test)]
mod store_tests {
    use crate::learning::{LearningStore, Proposal, Rule};

    fn store() -> (LearningStore, std::path::PathBuf) {
        let dir = std::env::temp_dir()
            .join("mecha-learning-test")
            .join(uuid::Uuid::new_v4().to_string());
        (LearningStore::open(dir.clone()).unwrap(), dir)
    }

    fn reflection(id: &str, extra: &str) -> String {
        format!(
            r#"{{"id":"{id}","domain":"behavior","session_id":"s1","trigger":"steer","context":"c","intervention":"i","reflexion_text":"t","created_at":"2026-09-01T00:00:00Z"{extra}}}"#
        )
    }

    #[test]
    fn marking_one_reflection_keeps_the_lines_this_build_cannot_read() {
        let (store, dir) = store();
        let path = dir.join("reflections.jsonl");
        let unreadable = r#"{"id":"r0","trigger":{"a kind":"from a newer build"}}"#;
        // Deliberately not this build's own rendering (spacing, key order),
        // so "kept" means byte for byte, not "rendered the same".
        let untouched = r#"{ "created_at":"2026-09-01T00:00:00Z", "id":"r2","domain":"behavior","session_id":"s1","trigger":"steer","context":"c","intervention":"i","reflexion_text":"t" }"#;
        let text = format!(
            "{unreadable}\n{}\n{untouched}\n",
            reflection("r1", r#","newer_field":{"kept":true}"#)
        );
        std::fs::write(&path, &text).unwrap();

        assert_eq!(
            store
                .mark_reflexions_processed(&["r1".into()], "run-1")
                .unwrap(),
            1
        );

        let after = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = after.lines().collect();
        assert_eq!(lines.len(), 3, "{after}");
        assert_eq!(
            lines[0], unreadable,
            "an unparseable line is kept where it was"
        );
        assert_eq!(
            lines[2], untouched,
            "an untouched line is kept byte for byte"
        );
        let marked: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(marked["is_processed"], true);
        assert_eq!(marked["leap_run_id"], "run-1");
        assert_eq!(marked["newer_field"], serde_json::json!({"kept": true}));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_rewrite_that_changes_nothing_does_not_write() {
        let (store, dir) = store();
        let path = dir.join("reflections.jsonl");
        std::fs::write(&path, format!("{}\n", reflection("r1", ""))).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(
            store
                .mark_reflexions_processed(&["absent".into()], "run-1")
                .unwrap(),
            0
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Undropping clears two fields this build knows; the unknown one stays.
    #[test]
    fn a_cleared_field_stays_cleared_beside_a_kept_unknown_one() {
        let (store, dir) = store();
        let path = dir.join("reflections.jsonl");
        let line = reflection(
            "r1",
            r#","dropped_at":"2026-09-02T00:00:00Z","dropped_reason":"noise","newer_field":1"#,
        );
        std::fs::write(&path, format!("{line}\n")).unwrap();
        store.restore_reflexion("r1").unwrap();
        let after: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert!(after.get("dropped_at").is_none(), "{after}");
        assert!(after.get("dropped_reason").is_none(), "{after}");
        assert_eq!(after["newer_field"], 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn rule(text: &str, id: &str) -> Rule {
        Rule {
            text: text.into(),
            id: Some(id.into()),
            ..Default::default()
        }
    }

    #[test]
    fn rewriting_learned_rules_keeps_what_a_newer_build_wrote() {
        let (store, dir) = store();
        store
            .write_learned_rules("behavior", &[rule("Old.", "r-a")])
            .unwrap();
        let path = dir.join("rules").join("behavior.learned.toml");
        let stored = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            format!(
                "format_note = \"from a newer build\"\n{}",
                stored.replace("id = \"r-a\"", "id = \"r-a\"\nnewer_field = \"kept\"",)
            ),
        )
        .unwrap();

        store
            .write_learned_rules("behavior", &[rule("Reworded.", "r-a"), rule("New.", "r-b")])
            .unwrap();

        let after: toml::Table = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["format_note"].as_str(), Some("from a newer build"));
        let rules = after["rules"].as_array().unwrap();
        assert_eq!(rules[0]["text"].as_str(), Some("Reworded."));
        assert_eq!(rules[0]["newer_field"].as_str(), Some("kept"));
        assert_eq!(rules[1]["text"].as_str(), Some("New."));
        assert!(rules[1].get("newer_field").is_none());
        let live = store.learned_rules("behavior").unwrap();
        assert_eq!(live.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The common case writes what it always wrote: the plain rendering.
    #[test]
    fn with_nothing_to_carry_the_rules_file_is_the_plain_rendering() {
        let (store, dir) = store();
        store
            .write_learned_rules("behavior", &[rule("One.", "r-a")])
            .unwrap();
        store
            .write_learned_rules("behavior", &[rule("One.", "r-a"), rule("Two.", "r-b")])
            .unwrap();
        let path = dir.join("rules").join("behavior.learned.toml");
        let plain = toml::to_string_pretty(&super::super::RulesFile {
            rules: vec![rule("One.", "r-a"), rule("Two.", "r-b")],
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), plain);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreadable_rules_file_is_refused_not_replaced() {
        let (store, dir) = store();
        let path = dir.join("rules").join("behavior.learned.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let broken = "[[rules]]\ntext = \"a hand edit, half done\n";
        std::fs::write(&path, broken).unwrap();
        let err = store
            .write_learned_rules("behavior", &[rule("New.", "r-b")])
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("refusing to replace"),
            "{err:#}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn proposal() -> Proposal {
        Proposal {
            id: "20260929T060000-p1".into(),
            domain: "behavior".into(),
            status: "pending".into(),
            reflexion_ids: vec!["r1".into()],
            rules_before: Vec::new(),
            rules: vec![rule("A rule.", "r-a")],
            evidence: "e".into(),
            created_at: "2026-09-29T06:00:00Z".into(),
            resolved_at: None,
            reason: None,
            scope: None,
        }
    }

    #[test]
    fn resolving_a_proposal_keeps_what_a_newer_build_wrote() {
        let (store, dir) = store();
        let p = proposal();
        store.write_proposal(&p).unwrap();
        let path = dir.join("proposals").join(format!("{}.json", p.id));
        let mut tree: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        tree["newer_field"] = serde_json::json!(["kept"]);
        std::fs::write(&path, serde_json::to_string_pretty(&tree).unwrap()).unwrap();

        let resolved = Proposal {
            status: "accepted".into(),
            resolved_at: Some("2026-09-29T07:00:00Z".into()),
            ..p
        };
        store.write_proposal(&resolved).unwrap();

        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["status"], "accepted");
        assert_eq!(after["newer_field"], serde_json::json!(["kept"]));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreadable_proposal_is_refused_not_replaced() {
        let (store, dir) = store();
        let p = proposal();
        let path = dir.join("proposals").join(format!("{}.json", p.id));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{\"id\": \"half").unwrap();
        assert!(store.write_proposal(&p).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"id\": \"half");
        std::fs::remove_dir_all(&dir).ok();
    }
}
