//! Deterministic post-principal checks over an isolated experiment's fixture state.
//! The assistant's prose and tool-call trace cannot satisfy these checks.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, io::Read, path::Path};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureCheck {
    /// Path relative to this trial's `fixtures/` directory.
    pub file: String,
    /// For a JSON file, the pointer to the array of records. Omit for JSONL.
    #[serde(default)]
    pub records_pointer: Option<String>,
    /// JSON pointers within each record and their exact required values.
    #[serde(default)]
    pub equals: BTreeMap<String, Value>,
    /// JSON pointers whose string values must contain these substrings.
    #[serde(default)]
    pub contains: BTreeMap<String, String>,
    /// Exact matching row count; catches omissions and duplicate sends.
    pub count: usize,
    /// An absent append-only ledger can mean zero rows only when explicitly declared.
    #[serde(default)]
    pub allow_missing: bool,
}
impl FixtureCheck {
    fn matching(&self, root: &Path) -> Result<usize> {
        let ctx = crate::tool::ToolCtx {
            workspace: root.to_path_buf(),
            ..Default::default()
        };
        let path = ctx.resolve(&self.file)?;
        use std::os::unix::fs::OpenOptionsExt;
        let f = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.allow_missing => {
                return Ok(0)
            }
            Err(e) => return Err(e.into()),
        };
        ensure!(
            f.metadata()?.is_file(),
            "fixture source must be a regular file"
        );
        let mut text = String::new();
        f.take(4 * 1024 * 1024 + 1).read_to_string(&mut text)?;
        ensure!(
            text.len() <= 4 * 1024 * 1024,
            "fixture source exceeds 4 MiB"
        );
        let records: Vec<Value> = match &self.records_pointer {
            Some(pointer) => serde_json::from_str::<Value>(&text)?
                .pointer(pointer)
                .and_then(Value::as_array)
                .context("records pointer is not an array")?
                .clone(),
            None => text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(serde_json::from_str)
                .collect::<std::result::Result<_, _>>()?,
        };
        Ok(records
            .iter()
            .filter(|r| {
                self.equals.iter().all(|(p, v)| r.pointer(p) == Some(v))
                    && self.contains.iter().all(|(p, v)| {
                        r.pointer(p)
                            .and_then(Value::as_str)
                            .is_some_and(|s| s.contains(v))
                    })
            })
            .count())
    }
    pub fn grade(&self, root: &Path) -> crate::eval::Check {
        let found = self.matching(root);
        crate::eval::Check {
            name: format!("fixture:{}", self.file),
            passed: found.as_ref().is_ok_and(|n| *n == self.count),
            detail: match found {
                Ok(n) => format!("expected {} matching records, observed {n}", self.count),
                Err(e) => format!("could not verify: {e:#}"),
            },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_delivery_content_and_duplicate_count_determine_the_grade() {
        let root =
            std::env::temp_dir().join(format!("mecha-fixture-grade-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let c = FixtureCheck {
            file: "sent.jsonl".into(),
            records_pointer: None,
            equals: BTreeMap::from([("/args/thread_id".into(), serde_json::json!("t-priya"))]),
            contains: BTreeMap::from([("/args/body_markdown".into(), "tracked changes".into())]),
            count: 1,
            allow_missing: false,
        };
        assert!(
            !c.grade(&root).passed,
            "a claimed send with no ledger is not success"
        );
        let row =
            "{\"args\":{\"thread_id\":\"t-priya\",\"body_markdown\":\"Bring tracked changes\"}}\n";
        std::fs::write(root.join("sent.jsonl"), row).unwrap();
        assert!(c.grade(&root).passed);
        std::fs::write(root.join("sent.jsonl"), format!("{row}{row}")).unwrap();
        assert!(!c.grade(&root).passed, "duplicate sends fail");
        std::fs::write(
            root.join("sent.jsonl"),
            row.replace("tracked changes", "unrelated text"),
        )
        .unwrap();
        assert!(
            !c.grade(&root).passed,
            "right recipient but wrong content fails"
        );
        let escaped = FixtureCheck {
            file: "../outside".into(),
            allow_missing: true,
            count: 0,
            ..c
        };
        assert!(!escaped.grade(&root).passed);
        std::fs::remove_dir_all(root).unwrap();
    }
}
