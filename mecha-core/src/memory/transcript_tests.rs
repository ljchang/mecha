use super::*;
use std::collections::BTreeSet;

pub(super) struct Fixture {
    pub dir: std::path::PathBuf,
    pub db: Memory,
}
impl Fixture {
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("memory-episodes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let db = Memory::create(&dir.join("test.memory-prototype.sqlite"), "test").unwrap();
        Self { dir, db }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
pub(super) fn access() -> Access {
    Access {
        domain: "test".into(),
        scopes: ["chat".into()].into_iter().collect(),
    }
}
pub(super) fn log(id: &str, texts: &[&str]) -> String {
    let mut lines = vec![
        serde_json::json!({"record":"meta","id":id,"created_at":"2026-01-01T00:00:00Z"})
            .to_string(),
    ];
    lines.extend(texts.iter().map(|text| serde_json::json!({"record":"message","role":"user","content":[{"type":"text","text":text}]}).to_string()));
    lines.join("\n") + "\n"
}

#[test]
fn exact_records_are_idempotent_and_append_in_source_order_without_invented_dates() {
    let mut f = Fixture::new();
    let text = log("s1", &["café\nsecond line", "A delivery is late."]);
    let first =
        f.db.import_session_jsonl(text.as_bytes(), "a", access().scopes)
            .unwrap();
    assert_eq!(first.inserted, 3);
    assert_eq!(first.unchanged, 0);
    let snapshot = f.db.snapshot().unwrap();
    let again =
        f.db.import_session_jsonl(text.as_bytes(), "a", access().scopes)
            .unwrap();
    assert_eq!(again.unchanged, 3);
    assert_eq!(f.db.snapshot().unwrap().sequence, snapshot.sequence);
    for (index, (raw, reference)) in text.split_inclusive('\n').zip(&first.records).enumerate() {
        let Some(Record::Evidence(e)) = f.db.read(reference, &access()).unwrap() else {
            panic!()
        };
        assert_eq!(e.text, raw);
        let meta = e.transcript.unwrap();
        assert_eq!(meta.line, index + 1);
        assert_eq!(meta.session_id, "s1");
        if index > 0 {
            assert!(e.occurred_at.is_none());
            assert_eq!(meta.speaker.as_deref(), Some("user"));
        }
    }
    let longer = log(
        "s1",
        &[
            "café\nsecond line",
            "A delivery is late.",
            "Check the alternative.",
        ],
    );
    let report =
        f.db.import_session_jsonl(longer.as_bytes(), "a", access().scopes)
            .unwrap();
    assert_eq!((report.inserted, report.unchanged), (1, 3));
    assert_eq!(&report.records[..3], &first.records);
}

#[test]
fn invalid_changed_and_truncated_imports_leave_no_partial_records() {
    let mut f = Fixture::new();
    let original = log("s1", &["original"]);
    f.db.import_session_jsonl(original.as_bytes(), "a", access().scopes)
        .unwrap();
    let before = f.db.snapshot().unwrap().sequence;
    for bad in [
        format!(
            "{}{{invalid}}\n",
            log("s1", &["original", "must not commit"])
        ),
        log("s1", &["changed"]),
        log("s1", &[]),
        original.trim_end().into(),
        "{\"record\":\"message\"}\n".into(),
    ] {
        assert!(f
            .db
            .import_session_jsonl(bad.as_bytes(), "a", access().scopes)
            .is_err());
        assert_eq!(f.db.snapshot().unwrap().sequence, before);
    }
}

#[test]
fn future_records_are_retained_and_rewrites_do_not_duplicate_original_messages() {
    let mut f = Fixture::new();
    let raw = format!(
        "{}{}\n{}\n{}\n",
        log("s1", &["original"]),
        r#"{"record":"future","timestamp":1234,"payload":"unrecognized"}"#,
        r#"{"record":"rewrite","messages":[{"role":"assistant","content":[{"type":"text","text":"generated summary"}]}]}"#,
        r#"{"record":"message","role":"assistant","content":[{"type":"future","text":"unrecognized block"},{"type":"text","text":"known block"}]}"#
    );
    let report =
        f.db.import_session_jsonl(raw.as_bytes(), "a", access().scopes)
            .unwrap();
    assert_eq!(report.inserted, 5);
    let Some(Record::Evidence(e)) = f.db.read(&report.records[3], &access()).unwrap() else {
        panic!()
    };
    assert!(e.text.contains("generated summary"));
    assert!(e.transcript.unwrap().fields.is_empty());
    let Some(Record::Evidence(e)) = f.db.read(&report.records[4], &access()).unwrap() else {
        panic!()
    };
    let fields = e.transcript.unwrap().fields;
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].pointer, "/content/1/text");
    assert_eq!(fields[0].text, "known block");
}

#[test]
fn tightening_scopes_and_forgetting_apply_to_reimports_and_old_reads() {
    let mut f = Fixture::new();
    let raw = log("s1", &["erase this synthetic phrase"]);
    let report =
        f.db.import_session_jsonl(raw.as_bytes(), "a", access().scopes)
            .unwrap();
    let old = report.records[1].clone();
    let private: BTreeSet<_> = ["chat".into(), "restricted".into()].into_iter().collect();
    f.db.import_session_jsonl(raw.as_bytes(), "a", private.clone())
        .unwrap();
    assert!(f.db.read(&old, &access()).unwrap().is_none());
    let permitted = Access {
        domain: "test".into(),
        scopes: private,
    };
    assert!(f.db.read(&old, &permitted).unwrap().is_some());
    f.db.forget(&transcript_source("a", "s1", 2)).unwrap();
    assert!(f.db.read(&old, &permitted).unwrap().is_none());
    let before = f.db.snapshot().unwrap().sequence;
    assert!(f
        .db
        .import_session_jsonl(raw.as_bytes(), "a", permitted.scopes)
        .is_err());
    assert_eq!(f.db.snapshot().unwrap().sequence, before);
}
