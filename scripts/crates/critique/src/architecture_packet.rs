//! What an architecture review freezes. The node runner's `application/architecture-packet.ts`.
//!
//! The repository's tree sampled by rule, `tree/MANIFEST.tsv` and `facts/`, and under `references/` every ADR in
//! force, `CONTEXT.md`, a `CONTEXT-MAP.md`, the README and the build configuration at the repository root, whose
//! layer-rule configs a finding about dependency direction needs. There is no artifact under review and no history.

use crate::adr_packet::{sample_tree, DocumentPacket};
use crate::architecture::{build_configs, BUILD_CONFIG_MAX_BYTES, READMES};
use crate::error::CritiqueError;
use crate::packet_files::{Packet, RepositoryPacket, UnfrozenFile};
use crate::paths::join;
use crate::recovery::{source_lines, source_sample};
use crate::recovery_packet::{decision_references, DecisionReferences};
use crate::repository_packet::PacketLimits;
use crate::runtime::Runtime;

/// The reference an architecture run's artifact carries: the repository itself, not a tracker item.
pub const ARCHITECTURE_ARTIFACT: &str = "architecture";

pub fn read_architecture_packet(runtime: &Runtime, root: &str, head: &str, limits: &PacketLimits) -> Result<Packet<RepositoryPacket>, CritiqueError> {
    let DecisionReferences { adrs, mut documents, maps } = decision_references(runtime, root)?;
    documents.extend(configuration(runtime, root)?);
    let text = documents.iter().map(|file| file.text.as_str()).collect::<Vec<_>>().join("\n\n");
    let packet = DocumentPacket { artifact_text: &text, files: &documents };
    match sample_tree(runtime, root, &packet, head, limits, Some(source_sample), true)? {
        Packet::Refused(refusal) => Ok(Packet::Refused(refusal)),
        Packet::Read(sampled) => Ok(Packet::Read(RepositoryPacket {
            adrs: Some(adrs),
            artifact_text: text,
            context_maps: Some(maps),
            files: sampled.files,
            reference: ARCHITECTURE_ARTIFACT.to_string(),
            repository: sampled.repository,
            source_lines: Some(source_lines(&sampled.tracked)),
            steps: None,
            stored_data_steps: None,
        })),
    }
}

/// The README and the root build configuration, each cut to its byte cap.
fn configuration(runtime: &Runtime, root: &str) -> Result<Vec<UnfrozenFile>, CritiqueError> {
    let present = runtime.dirs.list_files(root);
    let readme = READMES.iter().find(|name| present.iter().any(|file| file == *name));
    let sources: Vec<String> = readme.map(|name| name.to_string()).into_iter().chain(build_configs(&present)).collect();
    let mut files = Vec::new();
    for source in &sources {
        let text = runtime.files.read_text(&join(&[root, source])).map_err(CritiqueError::from_fs)?;
        let bytes = &text.as_bytes()[..text.len().min(BUILD_CONFIG_MAX_BYTES)];
        files.push(UnfrozenFile::new("reference", &join(&["references", source]), source, &String::from_utf8_lossy(bytes)));
    }
    Ok(files)
}
