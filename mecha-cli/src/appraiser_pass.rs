//! The quarantined-appraiser pass: one budgeted model call per already-built
//! appraisal.
//!
//! `mecha_core::appraisal::appraise_with_model` already does the isolated
//! call and its one retry; what this module adds is the budget and the
//! tally, on `appraisal_probe`'s own shape — a sibling paid pass over the
//! same walk, not a second implementation of either.

use anyhow::Result;
use mecha_core::appraisal::{apply_appraiser, Appraisal, AppraiserEvidence};
use mecha_core::provider::Provider;

/// What appraising one session found. Four ways to add no signed error,
/// counted apart — `appraisal_probe::Tally`'s own discipline: "the budget ran
/// out" and "the model looked and found nothing" are opposite findings, and
/// folding them into one counter hides which one a reader is looking at.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    /// Appraisals attempted — **not** model calls. `appraise_with_model`
    /// retries once on a malformed reply, so one driven appraisal can cost up
    /// to two calls; the retry is never separately charged to the budget,
    /// matching `appraisal_probe::Tally::driven`'s own unit ("arms actually
    /// driven", not requests sent).
    pub driven: usize,
    pub found_negative: usize,
    pub found_positive: usize,
    /// Driven, and the honest common answer: nothing further.
    pub found_nothing: usize,
    /// Driven and lost — a refusal, a parse failure surviving the retry, a
    /// transport error.
    pub failed: usize,
    /// Never looked at, because the budget ran out first.
    pub over_budget: usize,
}

impl Tally {
    pub fn add(&mut self, other: Tally) {
        self.driven += other.driven;
        self.found_negative += other.found_negative;
        self.found_positive += other.found_positive;
        self.found_nothing += other.found_nothing;
        self.failed += other.failed;
        self.over_budget += other.over_budget;
    }
}

/// Appraise one session if the budget allows.
///
/// `budget` is decremented only when the model is actually asked — never by a
/// skip — the same rule `appraisal_probe::probe_appraisal` follows, for the
/// same reason: a corpus that ran out of budget and one that was never going
/// to spend it read identically unless the two are kept apart.
pub async fn appraise_one(
    provider: &dyn Provider,
    model: &str,
    appraisal: &mut Appraisal,
    budget: &mut usize,
) -> Result<Tally> {
    let mut tally = Tally::default();
    if *budget == 0 {
        tally.over_budget += 1;
        return Ok(tally);
    }
    *budget -= 1;
    tally.driven += 1;

    let evidence = AppraiserEvidence::of(appraisal);
    match mecha_core::appraisal::appraise_with_model(provider, model, &evidence).await {
        Ok(v) => {
            match v.sign {
                None => tally.found_nothing += 1,
                Some(s) if s < 0.0 => tally.found_negative += 1,
                Some(_) => tally.found_positive += 1,
            }
            // The one place the model's own reasoning is ever read — printed
            // beside the tally for a human, on `appraiser_prompt`'s own
            // promise, and never carried any further: `apply_appraiser` has
            // no field for it, so it stops here.
            if let Some(reasoning) = &v.reasoning {
                eprintln!("· {}: {reasoning}", appraisal.session_id);
            }
            apply_appraiser(appraisal, v);
        }
        Err(e) => {
            // A skip is never evidence for either arm — `apply_appraiser` is
            // never called, so the appraisal stays exactly what the free
            // readout already produced.
            eprintln!("· {}: appraiser call failed: {e:#}", appraisal.session_id);
            tally.failed += 1;
        }
    }
    Ok(tally)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::appraisal::Affect;
    use mecha_core::learning::Origin;
    use mecha_core::message::{Block, CompletionResponse, Message, StopReason};
    use mecha_core::provider::StreamSink;

    fn appraisal() -> Appraisal {
        Appraisal {
            id: "s1".into(),
            session_id: "s1".into(),
            goals: Vec::new(),
            state: None,
            errors: Vec::new(),
            label: Affect::Neutral,
            origin: Origin::Clean,
            taint: Default::default(),
            created_at: "2026-08-27T00:00:00Z".into(),
            partial: false,
        }
    }

    /// Answers with one scripted reply and panics if asked a second time —
    /// proof that a zero budget never reaches the model at all.
    struct OneShot(std::sync::Mutex<Option<CompletionResponse>>);

    #[async_trait::async_trait]
    impl Provider for OneShot {
        fn id(&self) -> &str {
            "scripted"
        }
        fn default_model(&self) -> &str {
            "scripted-1"
        }
        async fn complete(
            &self,
            _req: &mecha_core::message::CompletionRequest,
            _sink: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            self.0
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| anyhow::anyhow!("asked a second time"))
        }
    }

    fn reply(text: &str) -> CompletionResponse {
        CompletionResponse {
            message: Message::assistant(vec![Block::text(text)]),
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
            refusal: None,
            model: "scripted-1".into(),
            malformed_tool_args: 0,
        }
    }

    #[tokio::test]
    async fn a_zero_budget_never_calls_the_model() {
        let provider = OneShot(std::sync::Mutex::new(Some(reply(
            r#"{"reasoning": "x", "verdict": "none"}"#,
        ))));
        let mut a = appraisal();
        let mut budget = 0usize;
        let tally = appraise_one(&provider, "scripted-1", &mut a, &mut budget)
            .await
            .unwrap();
        assert_eq!(tally.over_budget, 1);
        assert_eq!(tally.driven, 0);
        assert!(a.errors.is_empty(), "a skip must not touch the appraisal");
    }

    #[tokio::test]
    async fn a_nothing_further_verdict_is_tallied_apart_from_a_signed_one() {
        let provider = OneShot(std::sync::Mutex::new(Some(reply(
            r#"{"reasoning": "x", "verdict": "none"}"#,
        ))));
        let mut a = appraisal();
        let mut budget = 1usize;
        let tally = appraise_one(&provider, "scripted-1", &mut a, &mut budget)
            .await
            .unwrap();
        assert_eq!(tally.driven, 1);
        assert_eq!(tally.found_nothing, 1);
        assert_eq!(budget, 0, "the call was charged to the budget");
        assert!(a.errors.is_empty());
    }

    #[tokio::test]
    async fn a_signed_verdict_is_folded_in_and_tallied() {
        let provider = OneShot(std::sync::Mutex::new(Some(reply(
            r#"{"reasoning": "x", "verdict": "strongly_negative", "agency": "other"}"#,
        ))));
        let mut a = appraisal();
        let mut budget = 5usize;
        let tally = appraise_one(&provider, "scripted-1", &mut a, &mut budget)
            .await
            .unwrap();
        assert_eq!(tally.found_negative, 1);
        assert_eq!(a.errors.len(), 1);
        assert_eq!(a.label, Affect::Anger);
        assert_eq!(budget, 4);
    }

    #[tokio::test]
    async fn a_failed_call_is_tallied_and_changes_nothing() {
        let provider = OneShot(std::sync::Mutex::new(Some(reply("not json"))));
        let mut a = appraisal();
        let mut budget = 1usize;
        let tally = appraise_one(&provider, "scripted-1", &mut a, &mut budget)
            .await
            .unwrap();
        // The retry inside `appraise_with_model` consumes the one scripted
        // reply and then finds `OneShot` empty — itself proof the retry ran.
        assert_eq!(tally.failed, 1);
        assert!(a.errors.is_empty());
    }
}
