//! What an ADR review freezes. The node runner's `application/adr-packet.ts`.
//!
//! The records under review go to `artifact/` at their repository paths, and the glossary and every other record go
//! to `references/`. Every record is read whatever its status. With the code permission on, the packet is a
//! repository packet: beside the documents it holds `tree/MANIFEST.tsv`, a sample of the tree at the named commit
//! chosen by rule, and `facts/` with its index of what was sent and what the cap dropped.

use crate::check_adrs::{check_log, log_records};
use crate::code_nav;
use crate::decision_docs::{ADR_DIRECTORY, GLOSSARIES};
use crate::error::CritiqueError;
use crate::packet_files::{frozen_window, Packet, PreparedRepository, UnfrozenFile, TREE_DIR};
use crate::paths::{inside_repository, join};
use crate::ranking::{ranked_paths, ranking_note};
use crate::recovery::is_source_path;
use crate::repository::refusal;
use crate::repository_packet::{PacketLimits, TrackedFile};
use crate::repository_sample::{sample_repository, RepositoryFacts, SampleRequest, FACTS_INDEX_PATH, MANIFEST_PATH};
use crate::runtime::Runtime;
use crate::tracked_tree::read_tracked_tree;
use model_gateway::json;
use std::collections::HashSet;

/// The reference an ADR review's artifact carries: the repository's decision records, not a tracker item.
pub const ADR_ARTIFACT: &str = "adrs";

/// The packet path of the decision log's check.
pub const ADR_CHECK_PATH: &str = "check/adrs.json";

pub struct AdrRequest<'a> {
    /// The records under review; `None` means every record in `docs/adr/`.
    pub adrs: Option<&'a [String]>,
    /// The reference paths the caller names; `None` means the glossary and the records not under review.
    pub references: Option<&'a [String]>,
    /// Only when the repository allows code: the commit whose tree is sampled, and routing's packet limits.
    pub code: Option<(&'a str, &'a PacketLimits)>,
}

pub struct AdrPacket {
    /// What depth is chosen from: the records under review, as one text, and how many there are.
    pub artifact_text: String,
    pub adrs: usize,
    pub files: Vec<UnfrozenFile>,
    /// Only when the code permission let the tree be sampled.
    pub repository: Option<PreparedRepository>,
}

/// Reads what an ADR review freezes.
pub fn read_adr_packet(runtime: &Runtime, root: &str, request: &AdrRequest) -> Result<Packet<AdrPacket>, CritiqueError> {
    let records = log_records(runtime, root);
    let reviewed: Vec<String> = match request.adrs {
        None => records.clone(),
        Some(paths) => paths.iter().map(|path| inside_repository(root, path)).collect::<Result<_, _>>()?,
    };
    if reviewed.is_empty() {
        return Ok(Packet::Refused(refusal("no-adrs", &format!("found no decision records in {ADR_DIRECTORY}/ to review"))));
    }
    let references: Vec<String> = match request.references {
        Some(paths) => paths.iter().map(|path| inside_repository(root, path)).collect::<Result<_, _>>()?,
        None => glossary(runtime, root).into_iter().chain(records.iter().filter(|path| !reviewed.contains(path)).cloned()).collect(),
    };
    let artifact = reviewed.iter().map(|source| read_document(runtime, root, "artifact", source)).collect::<Result<Vec<_>, _>>()?;
    let check = check_log(runtime, root, &records)?;
    let mut documents = artifact.clone();
    for source in &references {
        documents.push(read_document(runtime, root, "reference", source)?);
    }
    documents.push(UnfrozenFile::new("check", ADR_CHECK_PATH, &format!("check-adrs {ADR_DIRECTORY}"), &format!("{}\n", json::pretty(&check))));
    let packet = AdrPacket { adrs: artifact.len(), artifact_text: artifact.iter().map(|file| file.text.as_str()).collect::<Vec<_>>().join("\n\n"), files: documents, repository: None };
    match request.code {
        None => Ok(Packet::Read(packet)),
        Some((head, limits)) => with_tree(runtime, root, packet, head, limits),
    }
}

/// A document of the repository, as a packet file of the folder's kind.
pub fn read_document(runtime: &Runtime, root: &str, folder: &'static str, source: &str) -> Result<UnfrozenFile, CritiqueError> {
    let place = if folder == "artifact" { "artifact" } else { "references" };
    let text = runtime.files.read_text(&join(&[root, source])).map_err(CritiqueError::from_fs)?;
    Ok(UnfrozenFile::new(folder, &join(&[place, source]), source, &text))
}

/// Adds the sampled tree, its manifest and the facts to a documents-only packet.
fn with_tree(runtime: &Runtime, root: &str, packet: AdrPacket, head: &str, limits: &PacketLimits) -> Result<Packet<AdrPacket>, CritiqueError> {
    let document = DocumentPacket { artifact_text: &packet.artifact_text, files: &packet.files };
    match sample_tree(runtime, root, &document, head, limits, None, false)? {
        Packet::Refused(refusal) => Ok(Packet::Refused(refusal)),
        Packet::Read(sampled) => Ok(Packet::Read(AdrPacket { files: sampled.files, repository: Some(sampled.repository), ..packet })),
    }
}

/// The first of `GLOSSARY.md` and `CONTEXT.md` the repository has, as decision docs pick it.
fn glossary(runtime: &Runtime, root: &str) -> Vec<String> {
    let present = runtime.dirs.list_files(root);
    GLOSSARIES.iter().find(|name| present.iter().any(|file| file == *name)).map(|name| name.to_string()).into_iter().collect()
}

/// A packet of documents, and the text that names the files of the tree to sample first.
pub struct DocumentPacket<'a> {
    pub artifact_text: &'a str,
    pub files: &'a [UnfrozenFile],
}

/// A documents packet with a sample of the repository's tree beside it.
pub struct SampledTree {
    pub files: Vec<UnfrozenFile>,
    pub repository: PreparedRepository,
    /// Every tracked path that may be sent, with its text, for what a stage measures of the repository.
    pub tracked: Vec<TrackedFile>,
}

/// Adds a repository packet's tree to a packet of documents: `tree/MANIFEST.tsv`, a sample of the tree at the commit
/// chosen by rule (the files the documents name first), and `facts/INDEX.md` with what was sent and what the cap
/// dropped. Vendored, generated and lock files are never read. `ranked` says whether code-nav's repository map
/// chooses and orders the files after the named ones.
pub fn sample_tree(
    runtime: &Runtime,
    root: &str,
    packet: &DocumentPacket,
    head: &str,
    limits: &PacketLimits,
    rest: Option<fn(&[TrackedFile]) -> Vec<String>>,
    ranked: bool,
) -> Result<Packet<SampledTree>, CritiqueError> {
    let commit = runtime.git.resolve_commit(root, head).map_err(CritiqueError::unexpected)?;
    let tree = read_tracked_tree(runtime, root, &commit, limits)?;
    let ranking = ranked.then(|| code_nav::rank(runtime, root));
    let sendable: HashSet<&str> = tree.tracked.iter().filter(|file| file.text.is_some()).map(|file| file.path.as_str()).collect();
    let rest_paths: Option<Vec<String>> = match &ranking {
        None => rest.map(|rest| rest(&tree.tracked)),
        Some(ranking) => {
            let first = ranked_paths(Some(ranking)).into_iter().filter(|path| sendable.contains(path.as_str()) && is_source_path(path));
            Some(first.chain(rest.map(|rest| rest(&tree.tracked)).unwrap_or_default()).collect())
        }
    };
    let documents: Vec<String> = packet.files.iter().map(|file| file.source.clone()).collect();
    let note = ranking.as_ref().map(ranking_note);
    let facts = RepositoryFacts::default();
    let request = SampleRequest {
        commit: &commit,
        limits,
        tracked: &tree.tracked,
        excluded: tree.excluded,
        documents: &documents,
        document_bytes: packet.files.iter().map(|file| file.text.len() as i64).sum(),
        artifact_text: packet.artifact_text,
        facts: &facts,
        rest: rest_paths.as_deref(),
        ranking_note: note.as_deref(),
    };
    let sample = match sample_repository(&request) {
        Ok(sample) => sample,
        Err(refusal) => return Ok(Packet::Refused(refusal)),
    };
    let mut files = packet.files.to_vec();
    files.push(UnfrozenFile::new("fact", MANIFEST_PATH, &commit, &sample.manifest));
    files.push(UnfrozenFile::new("fact", FACTS_INDEX_PATH, &commit, &sample.index));
    for file in &sample.tree {
        let mut frozen = UnfrozenFile::new("tree", &join(&[TREE_DIR, &file.path]), &file.path, &file.text);
        frozen.window = file.window.as_ref().map(|window| frozen_window(window, &[]));
        files.push(frozen);
    }
    let repository = PreparedRepository {
        cap_bytes: limits.cap_bytes,
        dropped_files: sample.dropped,
        head_commit: commit,
        packet_bytes: files.iter().map(|file| file.text.len()).sum(),
        tree_files: sample.tree.len(),
    };
    Ok(Packet::Read(SampledTree { files, repository, tracked: tree.tracked }))
}
