//! Context-only presentation. This is never a canonical record or storage format.
use super::{
    host_capture::{
        ArtifactCheck, CaptureIdentity, ConfirmedCriterion, OriginalDescriptor, ToolReceipt,
    },
    interpretation::{
        ClaimSource, Interpretation, SupportReport, ATOMIC_SUPPORT_PRODUCER,
        ATOMIC_SUPPORT_PROTOCOL,
    },
    sql::*,
    Access, Hit, Memory, Record, RevisionRef,
};
use anyhow::{ensure, Result};
use rusqlite::Connection;
use serde::{
    ser::{Error, SerializeSeq},
    Deserialize, Serialize, Serializer,
};

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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", deny_unknown_fields)]
pub(crate) enum ReaderRecord {
    #[serde(rename = "interpretation_reader_v1")]
    Interpretation(InterpretationReaderV1),
    #[serde(rename = "native_capture_reader_v1")]
    NativeCapture(NativeCaptureReaderV1),
}

/// Authenticated projection metadata only. Original passage bytes remain in
/// canonical Evidence hits; no prose is parsed or shortened for presentation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeCaptureReaderV1 {
    pub identity: CaptureIdentity,
    pub criterion: ConfirmedCriterion,
    pub originals: Vec<OriginalDescriptor>,
    pub receipts: Vec<ToolReceipt>,
    pub artifact: ArtifactCheck,
    #[serde(deserialize_with = "super::host_capture::strict_references")]
    pub consulted: Vec<RevisionRef>,
    pub occurred_at: Option<String>,
}
impl NativeCaptureReaderV1 {
    pub(crate) fn validate_manifest(&self, references: &[RevisionRef]) -> Result<()> {
        let mut originals = std::collections::BTreeSet::new();
        ensure!(
            self.originals
                .iter()
                .all(|d| references.contains(&d.reference)
                    && originals.insert((&d.reference.object, &d.reference.revision)))
                && self.consulted.iter().all(|r| references.contains(r)),
            "incomplete native reader manifest"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct PreparedRecords(Vec<(RevisionRef, Vec<u8>, ReaderRecord)>);
impl PreparedRecords {
    pub(super) fn prepare(conn: &Connection, hits: &[Hit]) -> Result<Self> {
        let references = hits.iter().map(|h| h.reference.clone()).collect::<Vec<_>>();
        let mut prepared = Vec::new();
        for hit in hits {
            let reader = match &hit.record {
                Record::Interpretation(value) => {
                    InterpretationReaderV1::from_canonical(value).map(ReaderRecord::Interpretation)
                }
                Record::Evidence(value) => {
                    super::host_capture::native_reader(conn, &hit.reference, value)?
                        .map(ReaderRecord::NativeCapture)
                }
                _ => None,
            };
            if let Some(reader) = reader {
                if let ReaderRecord::NativeCapture(value) = &reader {
                    value.validate_manifest(&references)?;
                }
                prepared.push((hit.reference.clone(), serde_json::to_vec(hit)?, reader));
            }
        }
        Ok(Self(prepared))
    }
}

pub(super) struct RecordsView<'a> {
    pub hits: &'a [Hit],
    pub prepared: &'a PreparedRecords,
}
impl Serialize for RecordsView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_records(self.hits, self.prepared, serializer)
    }
}

impl Memory {
    /// Wire metadata is not authority. Reconstruct it from current canonical
    /// sources and require every dependency, even if the wire omits its locator.
    /// Admission independently rechecks source authority before persistence.
    pub(crate) fn validate_native_context(
        &self,
        references: &[RevisionRef],
        readers: &[(RevisionRef, NativeCaptureReaderV1)],
        access: &Access,
    ) -> Result<()> {
        if readers.is_empty() {
            return Ok(());
        }
        self.check_access(access)?;
        self.live_identity()?;
        let tx = self.conn.unchecked_transaction()?;
        let checks = SourceChecks::new(self.source_guard.as_deref());
        for reference in references {
            ensure!(
                fresh(&tx, reference, i64::MAX)?
                    && authorized_with_guard(&tx, reference, access, Some(&checks))?,
                "native reader source unavailable"
            );
            ensure!(
                closure(&tx, reference)?
                    .iter()
                    .all(|r| references.contains(r)),
                "incomplete canonical native closure"
            );
        }
        for (reference, reader) in readers {
            let Some(Record::Evidence(canonical)) = lookup(&tx, reference)? else {
                anyhow::bail!("native reader source unavailable");
            };
            let expected = super::host_capture::native_reader(&tx, reference, &canonical)?
                .ok_or_else(|| anyhow::anyhow!("unauthenticated native reader"))?;
            ensure!(
                serde_json::to_vec(reader)? == serde_json::to_vec(&expected)?,
                "noncanonical native reader metadata"
            );
        }
        tx.commit()?;
        Ok(())
    }
}

pub(super) fn serialize_records<S: Serializer>(
    hits: &[Hit],
    prepared: &PreparedRecords,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct ReaderHit<'a> {
        reference: &'a super::RevisionRef,
        sequence: i64,
        record: &'a ReaderRecord,
        possible_time: bool,
        required_scopes: &'a std::collections::BTreeSet<String>,
        private_data: bool,
        untrusted_input: bool,
    }
    let mut sequence = serializer.serialize_seq(Some(hits.len()))?;
    for hit in hits {
        if let Some((_, canonical, reader)) = prepared
            .0
            .iter()
            .find(|(reference, _, _)| *reference == hit.reference)
        {
            let current = serde_json::to_vec(hit).map_err(S::Error::custom)?;
            if &current != canonical {
                return Err(S::Error::custom(
                    "canonical hit changed after reader preparation",
                ));
            }
            sequence.serialize_element(&ReaderHit {
                reference: &hit.reference,
                sequence: hit.sequence,
                record: reader,
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
