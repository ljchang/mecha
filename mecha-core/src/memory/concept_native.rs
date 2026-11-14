//! Canonical native originals, never caller-authored episode labels.
use super::*;
use rusqlite::{Connection, OptionalExtension};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Original {
    projection: RevisionRef,
    pub(super) episode: (String, String, String),
    pub(super) field: String,
    pub(super) text: String,
    pub(super) kind: host_capture::PassageKind,
}

fn original(conn: &Connection, reference: &RevisionRef) -> Result<Original> {
    let Some(Record::Evidence(e)) = lookup(conn, reference)? else {
        anyhow::bail!("native concept input must be original evidence")
    };
    ensure!(
        !matches!(
            e.source.connector.as_str(),
            "host-capture-projection-v1" | "host-capture-projection-v2"
        ),
        "native projection is not original text"
    );
    // The first immutable capture owns this original's original binding. Later
    // prefixes cannot replace its episode identity or resurrect its authority.
    let row: Option<(String,String,String,String)> = conn.query_row(
        "SELECT r.object_id,r.id,r.payload,h.originals FROM host_capture_receipt h JOIN revision r ON r.id=h.projection_revision WHERE h.state='active' AND EXISTS(SELECT 1 FROM json_each(h.originals) j WHERE json_extract(j.value,'$.revision')=?1 AND json_extract(j.value,'$.object')=?2) ORDER BY r.seq LIMIT 1",
        rusqlite::params![reference.revision,reference.object], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional()?;
    let (object, revision, payload, originals) =
        row.context("native original has no active canonical capture")?;
    let Record::Evidence(projection) = serde_json::from_str(&payload)? else {
        anyhow::bail!("invalid native projection")
    };
    let capture = host_capture::projection_input(conn, &projection)?;
    let originals: Vec<RevisionRef> = serde_json::from_str(&originals)?;
    ensure!(
        originals.len() == capture.originals.len(),
        "invalid native original descriptors"
    );
    let index = originals
        .iter()
        .position(|r| r == reference)
        .context("original missing from capture")?;
    let passage = &capture.originals[index];
    ensure!(
        serde_json::to_vec(&passage.source)? == serde_json::to_vec(&e.source)?
            && passage.text == e.text,
        "native original differs from capture"
    );
    Ok(Original {
        projection: RevisionRef { object, revision },
        episode: (
            capture.identity.domain,
            capture.identity.account,
            capture.identity.session_id,
        ),
        field: passage.pointer.clone(),
        text: passage.text.clone(),
        kind: passage.kind.clone(),
    })
}

type Sources = (BTreeMap<String, Original>, Vec<RevisionRef>);
pub(super) fn resolve_sources(conn: &Connection, references: &[RevisionRef]) -> Result<Sources> {
    ensure!(
        (2..=8).contains(&references.len()),
        "native concept requires 2..=8 originals"
    );
    let mut sources = BTreeMap::new();
    let mut dependencies = BTreeMap::new();
    for reference in references {
        let source = original(conn, reference)?;
        ensure!(
            sources
                .insert(reference.revision.clone(), source.clone())
                .is_none(),
            "duplicate native concept source"
        );
        for root in [reference, &source.projection] {
            for member in closure(conn, root)? {
                // Recaptured hypotheses cannot become independent observations.
                // This also keeps canonical validation nonrecursive and bounded.
                ensure!(
                    matches!(lookup(conn, &member)?, Some(Record::Evidence(_))),
                    "native concept induction excludes concept or interpretation ancestors"
                );
                let bound: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM external_binding WHERE revision=?1)",
                    [&member.revision],
                    |r| r.get(0),
                )?;
                ensure!(bound, "native concept requires sealed native lineage");
                dependencies.insert((member.object.clone(), member.revision.clone()), member);
            }
        }
        ensure!(
            dependencies.len() <= 64,
            "native concept lineage exceeds 64 records"
        );
    }
    Ok((sources, dependencies.into_values().collect()))
}

/// Local shape validation for host recapture. Authorization/currentness belongs
/// to the caller's full closure check. No model judgment becomes fact authority.
pub(in crate::memory) fn canonical(conn: &Connection, reference: &RevisionRef) -> Result<bool> {
    let check = || -> Result<()> {
        let Some(Record::Concept(c)) = lookup(conn, reference)? else {
            anyhow::bail!("not a concept")
        };
        let (native_sources, dependencies) = resolve_sources(conn, &c.consulted)?;
        let expected: BTreeSet<_> = dependencies
            .iter()
            .map(|r| (&r.object, &r.revision))
            .collect();
        let saved = super::dependencies(conn, &reference.revision)?;
        ensure!(
            saved
                .iter()
                .map(|r| (&r.object, &r.revision))
                .collect::<BTreeSet<_>>()
                == expected,
            "incomplete native concept dependencies"
        );
        for (text, max) in [
            (&c.name, 160),
            (&c.principle, 3000),
            (&c.applicability, 2000),
        ] {
            ensure!(
                !text.trim().is_empty() && text.len() <= max,
                "invalid concept text"
            );
        }
        ensure!(
            (2..=8).contains(&c.support.len()) && c.counterexamples.len() <= 8,
            "invalid concept citations"
        );
        let sources: Vec<_> = c
            .consulted
            .iter()
            .map(|r| {
                let Some(Record::Evidence(e)) = lookup(conn, r)? else {
                    anyhow::bail!("missing native original")
                };
                Ok((r.clone(), e))
            })
            .collect::<Result<_>>()?;
        for cited in [&c.support, &c.counterexamples] {
            let selected: Vec<_> = cited
                .iter()
                .map(|e| Selection {
                    source_id: e.evidence.revision.clone(),
                    field: e.field.clone(),
                    quote: e.quote.clone(),
                })
                .collect();
            let checked = excerpts(&sources, &selected, &native_sources)?;
            ensure!(
                serde_json::to_vec(&checked)? == serde_json::to_vec(cited)?,
                "invalid native concept quote"
            );
        }
        let episodes: BTreeSet<_> = c
            .support
            .iter()
            .map(|e| &native_sources[&e.evidence.revision].episode)
            .collect();
        ensure!(
            episodes.len() >= 2,
            "native concept requires independent episodes"
        );
        let own_scopes = required_scopes(conn, &reference.revision)?;
        for dependency in dependencies {
            ensure!(
                required_scopes(conn, &dependency.revision)?.is_subset(&own_scopes),
                "native concept lost inherited scopes"
            );
        }
        Ok(())
    };
    Ok(check().is_ok())
}
