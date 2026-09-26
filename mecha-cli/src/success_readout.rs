//! The owner's readout of the success set (row 2e-4a, R40):
//! `mecha sessions successes`, and the one-line count `sessions appraise`
//! prints beside its other store-wide readings.
//!
//! Read-only and free: it reads the outbox, the closure, workflow and
//! question stores and the session headers, derives the set
//! (`mecha_core::success::derive`), and writes nothing. It is the only
//! reader exemplars have — nothing serves one to a run.

use crate::logs::strip_ansi_and_controls as clean;
use anyhow::Result;
use mecha_core::success::{self, SessionIndex, Sources, Successes};
use std::collections::BTreeMap;
use std::path::Path;

/// Derive the set over `sources`, with the session store at `dir`. A session
/// store that cannot be listed makes every named session unknown to this
/// read, so the set is marked partial by name rather than read as though
/// no session were a test.
pub fn derive(dir: &Path, include_tests: bool, sources: &Sources<'_>) -> Successes {
    match SessionIndex::load(dir, include_tests) {
        Ok(index) => {
            let mut set = success::derive(sources, &index);
            // A header that did not read is a session this read cannot
            // place — possibly a smoke test, counted here as not one — so
            // the set is short, by name, as for every other store.
            if index.skipped > 0 {
                set.unreadable.push("session store");
            }
            set
        }
        Err(_) => {
            let mut set = success::derive(sources, &NoSessions);
            set.unreadable.push("session store");
            set
        }
    }
}

/// The session store could not be listed: every session is missing.
struct NoSessions;

impl success::SessionFacts for NoSessions {
    fn seen(&self, _: &str) -> success::Seen {
        success::Seen::Missing
    }
    fn completed(&self, _: &str) -> Option<bool> {
        None
    }
}

fn exemplar_origins(set: &Successes) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for e in &set.exemplars {
        let name = serde_json::to_value(e.origin)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".into());
        *out.entry(name).or_insert(0) += 1;
    }
    out
}

fn counts(map: &BTreeMap<impl std::fmt::Display, usize>) -> String {
    map.iter()
        .map(|(k, n)| format!("{k} {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The one-line summary, for `sessions appraise`.
pub fn line(set: &Successes) -> String {
    let mut s = format!("owner-verified successes: {} standing", set.standing.len());
    if !set.standing.is_empty() {
        s.push_str(&format!(" ({})", counts(&set.by_kind())));
    }
    s.push_str(&format!(
        ", {} withdrawn by a reopen, {} unknown; {} writing exemplar(s), served to no run",
        set.withdrawn.len(),
        set.unknown.len(),
        set.exemplars.len(),
    ));
    if set.hidden > 0 {
        s.push_str(&format!(
            "; {} in smoke-test or experiment sessions hidden",
            set.hidden
        ));
    }
    if set.partial() {
        s.push_str(&format!(
            " — partial: {} not fully read",
            set.unreadable.join(", ")
        ));
    }
    s
}

fn at(t: Option<chrono::DateTime<chrono::Utc>>) -> serde_json::Value {
    t.map(|t| serde_json::json!(t.to_rfc3339()))
        .unwrap_or(serde_json::Value::Null)
}

/// The summary as JSON, for `sessions appraise --json`.
pub fn summary_json(set: &Successes) -> serde_json::Value {
    serde_json::json!({
        "standing": set.standing.len(),
        "by_kind": set.by_kind(),
        "withdrawn": set.withdrawn.len(),
        "unknown": set.unknown.len(),
        "hidden": set.hidden,
        "partial": set.partial(),
        "unreadable": set.unreadable,
        "exemplars": {
            "count": set.exemplars.len(),
            "by_origin": exemplar_origins(set),
            "served": false,
        },
    })
}

/// The whole set as JSON; exemplars' arguments only with `exemplars`.
fn full_json(set: &Successes, exemplars: bool) -> serde_json::Value {
    let mut out = summary_json(set);
    let success = |s: &success::Success| {
        serde_json::json!({
            "kind": s.act.kind(),
            "pointer": s.act.pointer(),
            "sessions": s.sessions,
            "goal": s.goal.as_ref().map(ToString::to_string),
            "at": at(s.at),
        })
    };
    out["successes"] = set.standing.iter().map(success).collect();
    out["withdrawals"] = set
        .withdrawn
        .iter()
        .map(|w| {
            let mut v = success(&w.success);
            v["by"] = serde_json::json!(w.by);
            v["withdrawn_at"] = at(w.at);
            v
        })
        .collect();
    out["unknowns"] = set
        .unknown
        .iter()
        .map(|u| serde_json::json!({"pointer": u.pointer, "why": u.why}))
        .collect();
    if exemplars {
        out["exemplars"]["items"] = set
            .exemplars
            .iter()
            .map(|e| {
                serde_json::json!({
                    "item": e.item,
                    "tool": e.tool,
                    "args": e.args,
                    "session": e.session,
                    "situation": e.situation,
                    "origin": e.origin,
                    "sent_at": at(e.sent_at),
                })
            })
            .collect();
    }
    out
}

/// `mecha sessions successes`.
pub fn run(
    dir: &Path,
    json: bool,
    exemplars: bool,
    limit: Option<usize>,
    include_tests: bool,
) -> Result<()> {
    let stores = mecha_core::appraisal::Stores::load();
    let mut set = derive(dir, include_tests, &Sources::of(&stores));
    // Newest first, like every other listing here.
    // Undated rows last: `None < Some`, so a bare `Reverse` put them first,
    // where `-n` let them crowd out the newest (found on review).
    set.standing
        .sort_by_key(|s| (s.at.is_none(), std::cmp::Reverse(s.at)));
    set.withdrawn
        .sort_by_key(|w| (w.at.is_none(), std::cmp::Reverse(w.at)));
    set.exemplars
        .sort_by_key(|e| (e.sent_at.is_none(), std::cmp::Reverse(e.sent_at)));
    // The counts are the whole set's; `-n` caps only the listings.
    let summary = line(&set);
    let mut listed = full_json(&set, exemplars);
    if let Some(n) = limit {
        set.standing.truncate(n);
        set.withdrawn.truncate(n);
        set.exemplars.truncate(n);
        set.unknown.truncate(n);
        let capped = full_json(&set, exemplars);
        for key in ["successes", "withdrawals", "unknowns"] {
            listed[key] = capped[key].clone();
        }
        if exemplars {
            listed["exemplars"]["items"] = capped["exemplars"]["items"].clone();
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&listed)?);
        return Ok(());
    }
    println!("{summary}\n");
    let day = |t: Option<chrono::DateTime<chrono::Utc>>| {
        t.map(|t| t.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "????-??-??".into())
    };
    for s in &set.standing {
        println!(
            "  {}  {:<17} {}{}{}",
            day(s.at),
            s.act.kind(),
            clean(&s.act.pointer()),
            s.goal
                .as_ref()
                .map(|g| format!("  toward {}", clean(&g.to_string())))
                .unwrap_or_default(),
            if s.sessions.is_empty() {
                String::new()
            } else {
                format!("  session {}", clean(&s.sessions.join(", ")))
            },
        );
    }
    for w in &set.withdrawn {
        println!(
            "  {}  withdrawn         {} — taken back by {}",
            day(w.at),
            clean(&w.success.act.pointer()),
            clean(&w.by),
        );
    }
    for u in &set.unknown {
        println!("  unknown           {} — {}", clean(&u.pointer), u.why);
    }
    if exemplars {
        println!(
            "\nwriting exemplars — the drafts as sent, verbatim; nothing serves them to a run:"
        );
        for e in &set.exemplars {
            println!(
                "\n── outbox:{} · {} · {} · sent {}{}",
                clean(&e.item),
                clean(&e.tool),
                serde_json::to_value(e.origin)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                day(e.sent_at),
                e.session
                    .as_deref()
                    .map(|s| format!(" · session {}", clean(s)))
                    .unwrap_or_default(),
            );
            // Model-written text, printed to a terminal: each line of the
            // pretty JSON through the same filter every other readout of
            // model prose uses.
            let pretty = serde_json::to_string_pretty(&e.args).unwrap_or_default();
            for l in pretty.lines() {
                println!("   {}", clean(l));
            }
        }
    } else if !set.exemplars.is_empty() {
        println!("\n  (`--exemplars` prints each exemplar verbatim)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A short store is said by name on the line, and the exemplars are
    /// said to be served to no run — the shadow, stated where it is read.
    #[test]
    fn the_line_names_a_short_store_and_says_exemplars_are_not_served() {
        let set = success::derive(
            &Sources {
                questions_unreadable: true,
                ..Default::default()
            },
            &NoSessions,
        );
        let l = line(&set);
        assert!(l.contains("partial: question store not fully read"), "{l}");
        assert!(l.contains("served to no run"), "{l}");
        assert_eq!(summary_json(&set)["exemplars"]["served"], false);
        assert_eq!(summary_json(&set)["partial"], true);
    }

    /// A transcript whose header does not read is a short session store —
    /// it may have been a smoke test this read counts as not one.
    #[test]
    fn a_torn_transcript_makes_the_set_partial() {
        let dir = std::env::temp_dir().join(format!("mecha-torn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("s-torn.jsonl"), "{\"record\":\"me").unwrap();
        let set = derive(&dir, false, &Sources::default());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(set.unreadable.contains(&"session store"), "{set:?}");
    }

    /// A session store that cannot be listed is a short read, not a store
    /// with no tests in it.
    #[test]
    fn an_unlistable_session_store_makes_the_set_partial() {
        let file = std::env::temp_dir().join(format!("mecha-not-a-dir-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let set = derive(&file, false, &Sources::default());
        let _ = std::fs::remove_file(&file);
        assert!(set.unreadable.contains(&"session store"), "{set:?}");
    }
}
