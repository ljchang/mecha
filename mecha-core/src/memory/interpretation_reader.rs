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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion: Option<ConfirmedCriterion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_context: Option<super::chat_observation::ChatObservationContext>,
    pub originals: Vec<OriginalDescriptor>,
    pub receipts: Vec<ToolReceipt>,
    pub artifact: ArtifactCheck,
    #[serde(deserialize_with = "super::host_capture::strict_references")]
    pub consulted: Vec<RevisionRef>,
    pub occurred_at: Option<String>,
}
impl NativeCaptureReaderV1 {
    pub(crate) fn validate_manifest(&self, references: &[RevisionRef]) -> Result<()> {
        super::ordinary_observation::validate_context(
            &self.identity,
            self.criterion.as_ref(),
            self.chat_context.as_ref(),
        )?;
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
        self.validate_delivered_context(references, readers, access)
    }

    /// Reauthorize every delivered reference, including raw canonical packets.
    /// The legacy native-only wrapper above retains its original empty-reader policy.
    pub(crate) fn validate_delivered_context(
        &self,
        references: &[RevisionRef],
        readers: &[(RevisionRef, NativeCaptureReaderV1)],
        access: &Access,
    ) -> Result<()> {
        self.check_access(access)?;
        self.live_identity()?;
        let tx = self.conn.unchecked_transaction()?;
        let checks = SourceChecks::new(self.source_guard.as_deref());
        for reference in references {
            let canonical = closure(&tx, reference)?;
            // A purged revision has an empty closure. Its freshness and
            // authorization checks have nothing to examine.
            ensure!(
                canonical.contains(reference)
                    && fresh(&tx, reference, i64::MAX)?
                    && authorized_with_guard(&tx, reference, access, Some(&checks))?,
                "native reader source unavailable"
            );
            ensure!(
                canonical.iter().all(|r| references.contains(r)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{fixture, Capture};
    use std::fs;

    #[test]
    fn delivered_context_requires_the_exact_live_reference() {
        let dir = std::env::temp_dir().join(format!("memory-delivery-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let access = fixture::access();
        let mut memory =
            Memory::create(&dir.join("test.memory-prototype.sqlite"), &access.domain).unwrap();
        let mut capture = |id| {
            memory
                .capture(Capture {
                    source: fixture::source_key(id),
                    text: "fixture evidence".into(),
                    occurred_at: None,
                    scopes: access.scopes.clone(),
                    transcript: None,
                })
                .unwrap()
        };
        let live = capture("live");
        let missing = capture("withdrawn");
        memory.forget(&fixture::source_key("withdrawn")).unwrap();
        let wrong_object = RevisionRef {
            object: missing.object.clone(),
            revision: live.revision.clone(),
        };
        let empty_ok = memory.validate_delivered_context(&[], &[], &access).is_ok();
        let live_ok = memory
            .validate_delivered_context(&[live], &[], &access)
            .is_ok();
        let invalid_accepted = [missing, wrong_object]
            .iter()
            .map(|reference| {
                memory
                    .validate_delivered_context(std::slice::from_ref(reference), &[], &access)
                    .is_ok()
            })
            .collect::<Vec<_>>();
        drop(memory);
        fs::remove_dir_all(dir).unwrap();

        assert!(empty_ok);
        assert!(live_ok);
        assert_eq!(
            invalid_accepted,
            [false, false],
            "accepted missing or mismatched source"
        );
    }
}
