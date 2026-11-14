//! Shared reply parsing and ordered claim admission for interpretation producers.
//!
//! Pointer interpretation belongs to the caller: graph-local locators and
//! qualified memory source IDs must never be resolved in the same namespace.

use crate::appraisal_store::{ExpectedAct, Grounding, Judgment};
use crate::distill::Malformed;
use crate::message::{CompletionRequest, StopReason, Usage};
use crate::provider::Provider;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct ReplyClaim {
    pub statement: String,
    pub pointer: String,
    pub quote: String,
}

#[derive(Debug)]
pub(crate) struct ReplyDraft {
    pub interpretation: String,
    pub judgments: Vec<Judgment>,
    pub claims: Vec<ReplyClaim>,
    pub prediction: Option<String>,
    pub expected_act: Option<ExpectedAct>,
    pub unreadable_goals: usize,
    pub goal_hypotheses: Vec<String>,
    pub lessons: Vec<String>,
}

pub(crate) struct ReplyTurn {
    pub draft: Result<ReplyDraft, Malformed>,
    pub usage: Usage,
    pub elapsed: Duration,
    // Legacy accepts complete JSON at the cap; v2 independently refuses it.
    #[cfg_attr(not(feature = "memory-prototype"), allow(dead_code))]
    pub stop_reason: StopReason,
    #[cfg_attr(not(feature = "memory-prototype"), allow(dead_code))]
    pub reply_bytes: usize,
    /// Provider-reported identity, independent of the requested model alias.
    #[cfg_attr(not(feature = "memory-prototype"), allow(dead_code))]
    pub served_model: String,
}

pub(crate) async fn complete(
    provider: &dyn Provider,
    request: &CompletionRequest,
) -> anyhow::Result<ReplyTurn> {
    let started = Instant::now();
    let response = provider.complete(request, None).await?;
    let elapsed = started.elapsed();
    let text = response.message.text();
    let reply_bytes = text.len();
    let draft = match response.stop_reason {
        StopReason::Refusal => Err(Malformed::Refused),
        stop => parse_reply(&text).map_err(|m| {
            if stop == StopReason::MaxTokens
                && matches!(m, Malformed::NoJson | Malformed::Unreadable)
            {
                Malformed::CutOff
            } else {
                m
            }
        }),
    };
    Ok(ReplyTurn {
        draft,
        usage: response.usage,
        elapsed,
        stop_reason: response.stop_reason,
        reply_bytes,
        served_model: response.model,
    })
}

/// Original indices retained in their offered order and old-to-new support map.
pub(crate) struct ClaimAdmission {
    pub retained: Vec<usize>,
    pub grounding: Grounding,
    kept_at: BTreeMap<usize, usize>,
}

/// Ground before applying the retained-claim cap, preserving refusal precedence.
pub(crate) fn admit_claim_indices(
    offered: usize,
    max_claims: usize,
    mut check: impl FnMut(usize) -> Result<(), String>,
) -> ClaimAdmission {
    let mut admission = ClaimAdmission {
        retained: Vec::new(),
        grounding: Grounding {
            offered,
            ..Grounding::default()
        },
        kept_at: BTreeMap::new(),
    };
    for i in 0..offered {
        let reason = match check(i) {
            Ok(()) if admission.retained.len() >= max_claims => Some("over_cap".into()),
            Ok(()) => {
                admission.kept_at.insert(i, admission.retained.len());
                admission.retained.push(i);
                None
            }
            Err(reason) => Some(reason),
        };
        if let Some(reason) = reason {
            admission.grounding.dropped += 1;
            *admission.grounding.dropped_by.entry(reason).or_default() += 1;
        }
    }
    admission
}

/// Deduplicate in written order. Dropped support is grounding loss, not clipping.
pub(crate) fn remap_support(
    support: impl IntoIterator<Item = usize>,
    admission: &ClaimAdmission,
    max_support: usize,
    clipped: &mut bool,
) -> Vec<usize> {
    let mut seen = BTreeSet::new();
    let mut because = Vec::new();
    for i in support {
        if !seen.insert(i) {
            *clipped = true;
            continue;
        }
        if let Some(k) = admission.kept_at.get(&i).copied() {
            if because.len() >= max_support {
                *clipped = true;
                break;
            }
            because.push(k);
        }
    }
    because
}

pub(crate) fn parse_reply(text: &str) -> std::result::Result<ReplyDraft, Malformed> {
    use crate::appraisal_store::{Bearing, ExpectedAct, Judgment};
    let json = crate::eval::extract_json(text).ok_or(Malformed::NoJson)?;
    let v: Value = serde_json::from_str(&json).map_err(|_| Malformed::Unreadable)?;
    let obj = v.as_object().ok_or(Malformed::Unreadable)?;
    let present = |k: &str| obj.get(k).filter(|v| !v.is_null());
    let string = |k: &'static str| -> std::result::Result<Option<String>, Malformed> {
        match present(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(Malformed::Shape(k)),
        }
    };
    let list = |k: &'static str| -> std::result::Result<&[Value], Malformed> {
        match present(k) {
            None => Ok(&[]),
            Some(Value::Array(a)) => Ok(a.as_slice()),
            Some(_) => Err(Malformed::Shape(k)),
        }
    };
    let strings = |k: &'static str| -> std::result::Result<Vec<String>, Malformed> {
        list(k)?
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or(Malformed::Shape(k)))
            .collect()
    };

    let interpretation = string("interpretation")?.unwrap_or_default();
    if interpretation.trim().is_empty() {
        return Err(Malformed::NoInterpretation);
    }

    let mut unreadable_goals = 0usize;
    let mut judgments = Vec::new();
    for j in list("judgments")? {
        let j = j.as_object().ok_or(Malformed::Shape("judgments"))?;
        let goal = match j.get("goal").filter(|v| !v.is_null()) {
            None => None,
            Some(Value::String(g)) if g.trim().is_empty() => None,
            Some(Value::String(g)) => match g.trim().parse::<crate::goal::GoalRef>() {
                Ok(g) => Some(g),
                Err(_) => {
                    unreadable_goals += 1;
                    None
                }
            },
            Some(_) => return Err(Malformed::Shape("judgments.goal")),
        };
        let bearing = match j.get("bearing").filter(|v| !v.is_null()) {
            None => Bearing::Unknown,
            Some(Value::String(b)) => match b.trim().to_ascii_lowercase().as_str() {
                "good" => Bearing::Good,
                "bad" => Bearing::Bad,
                _ => Bearing::Unknown,
            },
            Some(_) => return Err(Malformed::Shape("judgments.bearing")),
        };
        let because = match j.get("because").filter(|v| !v.is_null()) {
            None => Vec::new(),
            Some(Value::Array(a)) => a
                .iter()
                .map(|i| {
                    i.as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .ok_or(Malformed::Shape("judgments.because"))
                })
                .collect::<std::result::Result<Vec<usize>, Malformed>>()?,
            Some(_) => return Err(Malformed::Shape("judgments.because")),
        };
        judgments.push(Judgment {
            goal,
            bearing,
            because,
        });
    }

    let mut claims = Vec::new();
    for c in list("claims")? {
        let c = c.as_object().ok_or(Malformed::Shape("claims"))?;
        let field = |k: &'static str| -> std::result::Result<String, Malformed> {
            match c.get(k).filter(|v| !v.is_null()) {
                None => Ok(String::new()),
                Some(Value::String(s)) => Ok(s.clone()),
                Some(_) => Err(Malformed::Shape("claims")),
            }
        };
        claims.push(ReplyClaim {
            statement: field("statement")?,
            pointer: field("pointer")?,
            quote: field("quote")?,
        });
    }

    Ok(ReplyDraft {
        interpretation,
        judgments,
        claims,
        prediction: string("prediction")?,
        expected_act: string("expected_act")?
            .as_deref()
            .and_then(ExpectedAct::parse),
        unreadable_goals,
        goal_hypotheses: strings("goal_hypotheses")?,
        lessons: strings("lessons")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_source_ids_survive_parsing_without_legacy_pointer_resolution() {
        let draft = parse_reply(r#"{"interpretation":"A grounded hypothesis","claims":[{"statement":"One","pointer":"source:episode-a/result:shared","quote":"the first source"},{"statement":"Two","pointer":"source:episode-b/result:shared","quote":"the second source"}]}"#).unwrap();
        assert_eq!(draft.claims[0].pointer, "source:episode-a/result:shared");
        assert_eq!(draft.claims[1].pointer, "source:episode-b/result:shared");
    }

    #[test]
    fn failed_claims_are_checked_before_the_retained_cap_and_support_keeps_order() {
        let admission = admit_claim_indices(5, 2, |i| match i {
            0 | 4 => Err("quote_not_found".into()),
            _ => Ok(()),
        });
        assert_eq!(admission.retained, vec![1, 2]);
        assert_eq!(admission.grounding.offered, 5);
        assert_eq!(admission.grounding.dropped, 3);
        assert_eq!(admission.grounding.dropped_by["quote_not_found"], 2);
        assert_eq!(admission.grounding.dropped_by["over_cap"], 1);
        let mut clipped = false;
        assert_eq!(
            remap_support([2, 0, 1, 2, 4], &admission, 2, &mut clipped),
            vec![1, 0]
        );
        assert!(clipped, "duplicate support flags clipping");
        let mut clipped = false;
        assert_eq!(
            remap_support([0, 4, 2], &admission, 2, &mut clipped),
            vec![1]
        );
        assert!(!clipped, "dropped support is grounding loss, not clipping");
    }
}
