//! Context-only presentation. This is never a canonical record or storage format.
use super::{
    host_capture::ConfirmedCriterion,
    interpretation::{
        ClaimSource, Interpretation, SupportReport, ATOMIC_SUPPORT_PRODUCER,
        ATOMIC_SUPPORT_PROTOCOL,
    },
    Hit, Record,
};
use anyhow::{ensure, Result};
use serde::{ser::SerializeSeq, Deserialize, Serialize, Serializer};

/// A compact atomic appraisal retains the complete checked report and original
/// locators. The omitted draft fields are redundant host renderings of it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InterpretationReaderV1 {
    pub context_key: String,
    pub criterion: ConfirmedCriterion,
    pub host_context_id: String,
    pub producer_version: String,
    pub claim_sources: Vec<ClaimSource>,
    pub support: SupportReport,
}
impl InterpretationReaderV1 {
    fn from_canonical(value: &Interpretation) -> Option<Self> {
        let support = value.draft.support.as_deref()?;
        if support.protocol != ATOMIC_SUPPORT_PROTOCOL
            || value.draft.producer_version != ATOMIC_SUPPORT_PRODUCER
            || support.atomic.is_none()
        {
            return None;
        }
        Some(Self {
            context_key: value.context_key.clone(),
            criterion: value.criterion.clone(),
            host_context_id: value.host_context_id.clone(),
            producer_version: value.draft.producer_version.clone(),
            claim_sources: value.claim_sources.clone(),
            support: support.clone(),
        })
    }

    /// Wire shape cannot promote a legacy support report to atomic status.
    /// Actual source authority still comes from canonical dependency admission.
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.producer_version == ATOMIC_SUPPORT_PRODUCER
                && self.support.protocol == ATOMIC_SUPPORT_PROTOCOL
                && self.support.atomic.is_some(),
            "invalid interpretation reader protocol"
        );
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", deny_unknown_fields)]
pub(crate) enum ReaderRecord {
    #[serde(rename = "interpretation_reader_v1")]
    Interpretation(InterpretationReaderV1),
}

pub(super) fn serialize_records<S: Serializer>(
    hits: &[Hit],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct ReaderHit<'a> {
        reference: &'a super::RevisionRef,
        sequence: i64,
        record: ReaderRecord,
        possible_time: bool,
        required_scopes: &'a std::collections::BTreeSet<String>,
        private_data: bool,
        untrusted_input: bool,
    }
    let mut sequence = serializer.serialize_seq(Some(hits.len()))?;
    for hit in hits {
        let compact = match &hit.record {
            Record::Interpretation(value) => InterpretationReaderV1::from_canonical(value),
            _ => None,
        };
        if let Some(reader) = compact {
            sequence.serialize_element(&ReaderHit {
                reference: &hit.reference,
                sequence: hit.sequence,
                record: ReaderRecord::Interpretation(reader),
                possible_time: hit.possible_time,
                required_scopes: &hit.required_scopes,
                private_data: hit.private_data,
                untrusted_input: hit.untrusted_input,
            })?;
        } else {
            sequence.serialize_element(hit)?;
        }
    }
    sequence.end()
}
