//! What the decisions stage freezes. The node runner's `application/decisions-packet.ts`.
//!
//! After `/grill-with-docs` the decision docs are the artifact, under `artifact/` at their repository paths. After
//! `/grill-me` the orchestrator's summary is the artifact, and the decision docs the repository already has are
//! reference material. The feature statement goes to `feature/`. No tracker is read: grilling comes before any spec.

use crate::decision_docs::{is_decision_record, ADR_DIRECTORY, DOMAIN_DOC, GLOSSARIES};
use crate::decision_docs_reader::resolve_decision_docs;
use crate::decisions::{decision_summary, feature_statement, DECISION_DOCS, DECISION_SUMMARY, FEATURE_PATH, SUMMARY_NAME};
use crate::error::CritiqueError;
use crate::adr_packet::read_document;
use crate::packet_files::{Packet, UnfrozenFile};
use crate::paths::{inside_repository, join};
use crate::repository::refusal;
use crate::runtime::Runtime;

pub struct DecisionsRequest<'a> {
    /// The orchestrator's short statement of the feature being grilled.
    pub feature: Option<&'a str>,
    /// The orchestrator's summary of the decisions, after `/grill-me`; `None` after `/grill-with-docs`.
    pub summary: Option<&'a str>,
    /// The decision docs the caller names; `None` means the repository's own.
    pub references: Option<&'a [String]>,
}

pub struct DecisionsPacket {
    /// The reference of the decisions under review, which is no tracker item.
    pub reference: &'static str,
    /// What depth is chosen from: the decisions under review, as one text.
    pub artifact_text: String,
    pub files: Vec<UnfrozenFile>,
    /// Whether the decisions under review include a decision record, which adds the stage's ADR critics.
    pub holds_adr: bool,
}

pub fn read_decisions_packet(runtime: &Runtime, root: &str, request: &DecisionsRequest) -> Result<Packet<DecisionsPacket>, CritiqueError> {
    let Some(feature) = request.feature else {
        return Err(CritiqueError::new("invalid-input", "decisions needs feature: a short statement of the feature the decisions are for"));
    };
    let feature = UnfrozenFile::new("feature", FEATURE_PATH, "feature-statement", &feature_statement(feature)?);
    let docs: Vec<String> = match request.references {
        None => resolve_decision_docs(runtime, root)?,
        Some(paths) => paths.iter().map(|path| inside_repository(root, path)).collect::<Result<_, _>>()?,
    };
    if let Some(summary) = request.summary {
        let text = decision_summary(summary)?;
        let summary = UnfrozenFile::new("artifact", &join(&["artifact", SUMMARY_NAME]), DECISION_SUMMARY, &text);
        let mut files = vec![feature, summary];
        for source in &docs {
            files.push(read_document(runtime, root, "reference", source)?);
        }
        return Ok(Packet::Read(packet(DECISION_SUMMARY, files, false)));
    }
    if docs.is_empty() {
        return Ok(Packet::Refused(refusal(
            "no-decision-docs",
            &format!("found no decision docs ({DOMAIN_DOC}, {} or {ADR_DIRECTORY}/); after /grill-me, pass the decisions as decision_summary", GLOSSARIES.join(", ")),
        )));
    }
    let mut files = vec![feature];
    for source in &docs {
        files.push(read_document(runtime, root, "artifact", source)?);
    }
    Ok(Packet::Read(packet(DECISION_DOCS, files, docs.iter().any(|doc| is_decision_record(doc)))))
}

fn packet(reference: &'static str, files: Vec<UnfrozenFile>, holds_adr: bool) -> DecisionsPacket {
    let artifact_text = files.iter().filter(|file| file.kind == "artifact").map(|file| file.text.as_str()).collect::<Vec<_>>().join("\n\n");
    DecisionsPacket { artifact_text, files, holds_adr, reference }
}
