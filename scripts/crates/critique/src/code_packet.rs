//! What a code stage freezes, computed at the commit under review by rule (ADR-0004). The node runner's
//! `application/code-packet.ts`.
//!
//! The diff from where the change left its base, without the hunks of generated, vendored and lock files; the ticket
//! or the spec's tickets, and the spec; `tree/REPO-MAP.md`, every source file's top-level symbols ranked by
//! references; and a capped `tree/` read from git's object store, filled with the context slices around the change
//! first, then the changed files, their importers and the rest of the tree, with `facts/INDEX.md` recording every drop.

use crate::code_change::read_change;
use crate::code_nav;
use crate::code_sample::{sample_code_tree, slice_reason, CodeSampleRequest};
use crate::context_slices::{context_slices, ChangedFile};
use crate::error::CritiqueError;
use crate::packet_files::{frozen_window, Packet, UnfrozenFile, TREE_DIR};
use crate::paths::join;
use crate::ranking::{ranked_paths, ranking_note, RankedFile, RankedSymbol, RepoRanking};
use crate::repo_map::{build_repo_map, render_code_nav_map, render_repo_map, SourceGraph, REPO_MAP_PATH};
use crate::repository::Resolved;
use crate::repository_packet::PacketLimits;
use crate::repository_sample::FACTS_INDEX_PATH;
use crate::runtime::Runtime;
use crate::tracked_tree::read_tracked_tree;
use crate::tracker_resolver::{ArtifactRead, TicketsRead, TrackedArtifact, Tracker};
use crate::triage::CodeChange;
use std::collections::HashSet;

pub struct CodeRequest<'a> {
    pub stage: &'a str,
    /// The spec, named the way its tracker names it.
    pub spec: &'a str,
    /// The ticket a ticket review delivers; a feature review reads every ticket of the spec.
    pub ticket: Option<&'a str>,
    /// The commit the change starts from, or a branch it left.
    pub base: Option<&'a str>,
    /// The commit under review.
    pub head: &'a str,
    pub comments: bool,
    /// Routing's packet limits.
    pub limits: &'a PacketLimits,
}

/// One context slice of a code packet: lines of a tree file, and why they are sent.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeSliceSource {
    pub first: usize,
    pub last: usize,
    pub path: String,
    pub reason: String,
}

/// What a code packet freezes of the commits.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeSourceOut {
    pub base: String,
    pub base_commit: String,
    pub changed_files: Vec<String>,
    pub head: String,
    pub head_commit: String,
    pub slices: Vec<CodeSliceSource>,
}

pub struct CodePacket {
    pub files: Vec<UnfrozenFile>,
    pub code: CodeSourceOut,
    /// The change as the risk rules read it, for the stage's depth signals.
    pub change: CodeChange,
    /// What the stage critiques: the ticket for a ticket review, the spec for a feature review.
    pub artifact: TrackedArtifact,
    /// The spec's reference, which a ticket review's flow defaults to.
    pub spec_reference: String,
    /// How many files of the tree the cap left out in whole or in part.
    pub dropped_files: usize,
    pub cap_bytes: f64,
}

/// A ticket's packet file.
pub fn ticket_file(name: &str, reference: &str, text: &str) -> UnfrozenFile {
    UnfrozenFile::new("ticket", &format!("tickets/{name}"), reference, text)
}

/// The tickets of a code packet: the one a ticket review names, or else every ticket of the spec.
struct Tickets {
    files: Vec<UnfrozenFile>,
    ticket: Option<TrackedArtifact>,
}

fn tickets(runtime: &Runtime, tracker: &Tracker, request: &CodeRequest, reference: Option<&str>) -> Result<Packet<Tickets>, CritiqueError> {
    if let Some(reference) = reference {
        return Ok(match tracker.artifact(runtime, reference, request.comments)? {
            ArtifactRead::Refused(refusal) => Packet::Refused(refusal),
            ArtifactRead::Read(ticket) => Packet::Read(Tickets { files: vec![ticket_file(&ticket.name, &ticket.reference, &ticket.text)], ticket: Some(ticket) }),
        });
    }
    Ok(match tracker.tickets(runtime, request.spec)? {
        TicketsRead::Refused(refusal) => Packet::Refused(refusal),
        TicketsRead::Read(read) => Packet::Read(Tickets { files: read.iter().map(|tracked| ticket_file(&tracked.name, &tracked.ticket.reference, &tracked.ticket.text)).collect(), ticket: None }),
    })
}

/// code-nav's map without the files and symbols the tree does not hold, which the map read from the working tree.
fn only_tracked(ranking: &RepoRanking, held: &HashSet<&str>) -> Option<(Vec<RankedFile>, Vec<RankedSymbol>)> {
    match ranking {
        RepoRanking::Ranked { files, symbols } => Some((
            files.iter().filter(|file| held.contains(file.path.as_str())).cloned().collect(),
            symbols.iter().filter(|symbol| held.contains(symbol.path.as_str())).cloned().collect(),
        )),
        RepoRanking::Unavailable { .. } => None,
    }
}

pub fn read_code_packet(runtime: &Runtime, repository: &Resolved, tracker: &Tracker, request: &CodeRequest) -> Result<Packet<CodePacket>, CritiqueError> {
    let (limits, stage) = (request.limits, request.stage);
    let Some(base) = request.base else {
        return Err(CritiqueError::new("invalid-input", format!("{stage} needs base: the commit or branch the change is diffed against")));
    };
    if stage == "ticket-review" && request.ticket.is_none() {
        return Err(CritiqueError::new("invalid-input", "ticket-review needs ticket: the ticket the change delivers"));
    }
    let root = repository.root.as_str();
    let read = read_change(runtime, root, base, request.head, limits)?;
    let (base_commit, head_commit) = (read.base_commit.clone(), read.head_commit.clone());
    let spec = match tracker.artifact(runtime, request.spec, request.comments)? {
        ArtifactRead::Refused(refusal) => return Ok(Packet::Refused(refusal)),
        ArtifactRead::Read(spec) => spec,
    };
    let tickets = match tickets(runtime, tracker, request, if stage == "ticket-review" { request.ticket } else { None })? {
        Packet::Refused(refusal) => return Ok(Packet::Refused(refusal)),
        Packet::Read(tickets) => tickets,
    };
    let tree = read_tracked_tree(runtime, root, &head_commit, limits)?;
    let texts: Vec<(String, String)> = tree.tracked.iter().filter_map(|file| file.text.clone().map(|text| (file.path.clone(), text))).collect();
    let held: HashSet<&str> = texts.iter().map(|(path, _)| path.as_str()).collect();
    let graph = SourceGraph::build(&texts);
    let changed: Vec<String> = read.change.paths.iter().filter(|path| held.contains(path.as_str())).cloned().collect();
    let changed_files: Vec<ChangedFile> = changed
        .iter()
        .map(|path| ChangedFile {
            before: runtime.git.file_at(root, &base_commit, path).ok(),
            path: path.clone(),
            text: texts.iter().find(|(held, _)| held == path).map(|(_, text)| text.clone()).unwrap_or_default(),
        })
        .collect();
    let mut documents = tickets.files.clone();
    documents.push(UnfrozenFile::new("spec", &join(&["spec", &spec.name]), &spec.reference, &spec.text));
    let ranking = code_nav::rank(runtime, root);
    // code-nav's map, limited to the files this tree holds; the runner's own scan when code-nav gave none.
    let repo_map_cap = limits.repo_map_cap_bytes as i64;
    let repo_map = match only_tracked(&ranking, &held) {
        Some((files, symbols)) if !files.is_empty() => render_code_nav_map(&files, &symbols, &head_commit, repo_map_cap),
        _ => render_repo_map(&build_repo_map(&graph), &head_commit, repo_map_cap),
    };
    let slices = context_slices(&graph, &changed_files);
    let mut importers: Vec<String> = Vec::new();
    for path in &changed {
        for importer in graph.importers(path) {
            if !importers.contains(&importer) && !changed.contains(&importer) {
                importers.push(importer);
            }
        }
    }
    importers.sort_by(|a, b| crate::code_facts::code_unit_order(a, b));
    let named_text = documents.iter().map(|file| file.text.as_str()).collect::<Vec<_>>().join("\n");
    let ranked = ranked_paths(Some(&ranking));
    let map_note = if repo_map.source == "code-nav" { format!("`{REPO_MAP_PATH}` is built from that map.") } else { format!("`{REPO_MAP_PATH}` comes from the runner's own scan.") };
    let note = format!("{} {map_note}", ranking_note(&ranking));
    let sample_request = CodeSampleRequest {
        commit: &head_commit,
        limits,
        tracked: &tree.tracked,
        excluded: tree.excluded,
        left_out: &read.left_out,
        diff: &read.diff,
        document_bytes: documents.iter().map(|file| file.text.len() as i64).sum(),
        repo_map: &repo_map,
        changed: &changed,
        importers: &importers,
        slices: &slices,
        named_text: &named_text,
        ranked: Some(&ranked),
        ranking_note: Some(&note),
    };
    let sample = match sample_code_tree(&sample_request) {
        Ok(sample) => sample,
        Err(refusal) => return Ok(Packet::Refused(refusal)),
    };
    let mut files = vec![UnfrozenFile::new("diff", "diff/change.diff", &format!("{base_commit}..{head_commit}"), &sample.diff)];
    files.extend(documents);
    files.push(UnfrozenFile::new("fact", REPO_MAP_PATH, &head_commit, &repo_map.text));
    files.push(UnfrozenFile::new("fact", FACTS_INDEX_PATH, &head_commit, &sample.index));
    for file in &sample.tree {
        let mut frozen = UnfrozenFile::new("tree", &join(&[TREE_DIR, &file.path]), &file.path, &file.text);
        frozen.window = file.window.as_ref().map(|window| frozen_window(window, &slices));
        files.push(frozen);
    }
    let sliced = sample.slices.iter().map(|slice| CodeSliceSource { first: slice.first, last: slice.last, path: join(&[TREE_DIR, &slice.path]), reason: slice_reason(slice) }).collect();
    Ok(Packet::Read(CodePacket {
        artifact: tickets.ticket.unwrap_or(spec.clone()),
        cap_bytes: limits.cap_bytes,
        change: read.change,
        code: CodeSourceOut {
            base: base.to_string(),
            base_commit,
            changed_files: changed.iter().map(|path| join(&[TREE_DIR, path])).collect(),
            head: request.head.to_string(),
            head_commit,
            slices: sliced,
        },
        dropped_files: sample.dropped,
        files,
        spec_reference: spec.reference,
    }))
}
