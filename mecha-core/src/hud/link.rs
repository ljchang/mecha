//! The served half of a typed reference (`GITHUB-DASHBOARD-DESIGN.md` §4):
//! which rows of a dataset may draw their GitHub reference as an anchor.
//!
//! The spec gate is the control — a reference names three fields and
//! nothing else (`spec.rs`). This is the check behind it, made where the
//! dataset is served and again in the browser (`web/src/lib/hud/hud.js`), so
//! a row is linked only when:
//!
//! - its owner and repository match GitHub's name grammar (`.` and `..`
//!   refused as a whole name, `%` and `/` refused anywhere) and its number is
//!   a positive integer — so no value can walk the fixed path or smuggle an
//!   escape into it; and
//! - the pair is a repository the collector both saw and the owner selected
//!   (`github::Watched`, inventory ∩ selection). Without this an issue's text
//!   still could not move the link, but a poisoned row could point a click at
//!   a repository of a stranger's choosing.
//!
//! What it returns is a list of the triples that passed, beside the dataset.
//! A row whose triple is not in it renders as text. **Absence is refusal**:
//! a dataset served without this list — a published board's, a proposal's,
//! an older server's — links nothing, so the renderer fails closed by
//! construction rather than by remembering to.
//!
//! No address is built here. The destination is the renderer's one fixed
//! template; the server only says which values it may be built from.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;

use super::spec::{GithubRef, Spec};
use super::store::Dataset;
use crate::github::{valid_owner, valid_repo_name};

/// One reference a table over the dataset makes, and the rows that may
/// draw it as an anchor — by value, not by index, so the renderer's own
/// filtering and ordering cannot misalign them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Linkable {
    /// The reference as the spec writes it: `ref`, `owner`, `repo`,
    /// `number`. The renderer matches its column to this by all four.
    #[serde(flatten)]
    pub reference: GithubRef,
    /// `[owner, repo, number]` of each row that passed, once each.
    pub allowed: Vec<(String, String, u64)>,
}

/// An issue number: a JSON integer above zero. Not a string that looks like
/// one — a loader declares the column an integer (`cross_check`).
pub fn issue_number(v: &Value) -> Option<u64> {
    v.as_u64().filter(|n| *n > 0)
}

/// The values of one row that may build a GitHub reference: both names in
/// GitHub's grammar, the number positive. The containment question is the
/// caller's (`watched`).
pub fn github_parts(owner: &Value, repo: &Value, number: &Value) -> Option<(String, String, u64)> {
    let owner = owner.as_str().filter(|s| valid_owner(s))?;
    let repo = repo.as_str().filter(|s| valid_repo_name(s))?;
    Some((owner.to_string(), repo.to_string(), issue_number(number)?))
}

/// For each GitHub reference the checked spec makes over `dataset`, the rows
/// whose values pass the grammar and name a repository `watched` admits.
/// Empty when the spec makes no reference over it — which columns are a
/// reference comes from the spec's type, never from the rows.
pub fn github_linkable(
    spec: &Spec,
    dataset: &Dataset,
    watched: &dyn Fn(&str, &str) -> bool,
) -> Vec<Linkable> {
    let col = |name: &str| dataset.columns.iter().position(|c| c.name == name);
    spec.github_refs(&dataset.name)
        .into_iter()
        .map(|r| {
            let mut allowed = BTreeSet::new();
            // A column the dataset does not carry (a loader edited since the
            // spec was checked) links nothing.
            if let (Some(o), Some(p), Some(n)) = (col(&r.owner), col(&r.repo), col(&r.number)) {
                for row in &dataset.rows {
                    let cell = |i: usize| row.get(i).unwrap_or(&Value::Null);
                    if let Some(parts) = github_parts(cell(o), cell(p), cell(n)) {
                        if watched(&parts.0, &parts.1) {
                            allowed.insert(parts);
                        }
                    }
                }
            }
            Linkable {
                reference: r.clone(),
                allowed: allowed.into_iter().collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::loader::{Column, ColumnType};
    use serde_json::json;

    fn spec() -> Spec {
        Spec::parse(
            &json!({
                "version": 1, "title": "New", "datasets": ["new_items"],
                "panels": [{ "type": "table", "title": "New", "dataset": "new_items",
                  "columns": ["title",
                    { "ref": "github", "owner": "owner", "repo": "name", "number": "number" }] }]
            })
            .to_string(),
        )
        .unwrap()
    }

    fn dataset(rows: Vec<Value>) -> Dataset {
        let col = |name: &str, kind| Column {
            name: name.into(),
            kind,
        };
        Dataset {
            name: "new_items".into(),
            generated_at: chrono::Utc::now(),
            generation: 1,
            loader_digest: "fixture".into(),
            columns: vec![
                col("title", ColumnType::String),
                col("owner", ColumnType::String),
                col("name", ColumnType::String),
                col("number", ColumnType::Integer),
            ],
            rows: rows
                .into_iter()
                .map(|r| r.as_array().unwrap().clone())
                .collect(),
            external: true,
        }
    }

    fn lab_only(owner: &str, repo: &str) -> bool {
        owner == "example-lab" && repo == "tool-a"
    }

    fn allowed(rows: Vec<Value>) -> Vec<(String, String, u64)> {
        let got = github_linkable(&spec(), &dataset(rows), &lab_only);
        assert_eq!(got.len(), 1);
        got.into_iter().next().unwrap().allowed
    }

    /// A row in the watched set links; one outside it — a repository a
    /// poisoned row names, well-formed or not — does not.
    #[test]
    fn only_a_watched_repository_links() {
        let got = allowed(vec![
            json!(["A bug", "example-lab", "tool-a", 12]),
            json!(["Elsewhere", "stranger-org", "bait", 3]),
            json!(["Same owner", "example-lab", "tool-b", 4]),
        ]);
        assert_eq!(got, vec![("example-lab".into(), "tool-a".into(), 12)]);
    }

    /// A value outside the grammar never links, even if the watched set were
    /// to admit it: the path cannot be walked or escaped.
    #[test]
    fn a_value_outside_the_grammar_never_links() {
        let everything = |_: &str, _: &str| true;
        for row in [
            json!(["x", "example-lab", "..", 1]),
            json!(["x", "example-lab", ".", 1]),
            json!(["x", "example-lab", "%2e%2e", 1]),
            json!(["x", "example-lab", "tool-a/../../x", 1]),
            json!(["x", "..", "tool-a", 1]),
            json!(["x", "example-lab", "tool-a?x=1", 1]),
            json!(["x", "example-lab", "tool-a#frag", 1]),
            json!(["x", "example-lab", "", 1]),
            json!(["x", "example-lab", "tool-a", 0]),
            json!(["x", "example-lab", "tool-a", -1]),
            json!(["x", "example-lab", "tool-a", "12"]),
            json!(["x", "example-lab", "tool-a", 1.5]),
            json!(["x", "example-lab", "tool-a", null]),
            json!(["x", null, "tool-a", 1]),
        ] {
            let got = github_linkable(&spec(), &dataset(vec![row.clone()]), &everything);
            assert!(got[0].allowed.is_empty(), "{row} linked");
        }
    }

    /// No reference in the spec, no list: the rows cannot ask for one.
    #[test]
    fn a_spec_with_no_reference_links_nothing() {
        let plain = Spec::parse(
            &json!({
                "version": 1, "title": "New", "datasets": ["new_items"],
                "panels": [{ "type": "table", "title": "New", "dataset": "new_items",
                  "columns": ["title", "owner", "name", "number"] }]
            })
            .to_string(),
        )
        .unwrap();
        let d = dataset(vec![json!(["A bug", "example-lab", "tool-a", 12])]);
        assert!(github_linkable(&plain, &d, &lab_only).is_empty());
    }

    /// Serialised as the renderer matches it: the reference's own four keys
    /// beside `allowed`.
    #[test]
    fn the_list_carries_the_reference_it_answers() {
        let got = github_linkable(
            &spec(),
            &dataset(vec![json!(["A bug", "example-lab", "tool-a", 12])]),
            &lab_only,
        );
        assert_eq!(
            serde_json::to_value(&got).unwrap(),
            json!([{ "ref": "github", "owner": "owner", "repo": "name", "number": "number",
                     "allowed": [["example-lab", "tool-a", 12]] }])
        );
    }
}
