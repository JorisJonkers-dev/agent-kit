//! What a decision-recovery run freezes. The node runner's `application/recovery-packet.ts`.
//!
//! The repository's tree sampled by rule, `tree/MANIFEST.tsv` and `facts/`, every ADR in force, `CONTEXT.md` and a
//! `CONTEXT-MAP.md`, the last three under `references/`. There is no artifact under review: the decisions are what
//! the critics find. Only when the allowlist entry allows history does the packet also hold `history/log.txt`, the
//! commit messages for the sampled files, and `history/issues/`, the issues those messages cite where the tracker can
//! be read. Without that permission no commit message or issue text is read.

use crate::adr_packet::{sample_tree, DocumentPacket, SampledTree};
use crate::check_adrs::log_records;
use crate::decision_docs::{in_force, CONTEXT_MAP, GLOSSARIES};
use crate::error::CritiqueError;
use crate::packet_files::{Packet, PreparedRepository, RepositoryPacket, UnfrozenFile};
use crate::paths::join;
use crate::recovery::{
    cited_issues, issue_reference, render_history_log, source_lines, source_sample, HISTORY_ISSUES_DIR, HISTORY_ISSUE_BYTES, HISTORY_LOG_PATH, HISTORY_MAX_COMMITS,
    HISTORY_MAX_PATHS, HISTORY_RESERVE_BYTES,
};
use crate::repository_packet::PacketLimits;
use crate::runtime::Runtime;
use crate::tracker_resolver::{ArtifactRead, Tracker};

/// The reference a recovery run's artifact carries: the repository itself, not a tracker item.
pub const RECOVERY_ARTIFACT: &str = "recovery";

pub struct RecoveryRequest<'a, 'b> {
    /// The commit whose tree is sampled.
    pub head: &'a str,
    /// Routing's packet limits.
    pub limits: &'a PacketLimits,
    /// Only when the allowlist entry allows history: the tracker to read the issues the commit messages cite from,
    /// `None` inside it when the repository's tracker cannot be read.
    pub history: Option<Option<&'a Tracker<'b>>>,
}

/// The decision records a repository packet holds as references: the glossary, a context map and every ADR in
/// force, with how many of the last two.
pub struct DecisionReferences {
    pub documents: Vec<UnfrozenFile>,
    pub adrs: usize,
    pub maps: usize,
}

pub fn decision_references(runtime: &Runtime, root: &str) -> Result<DecisionReferences, CritiqueError> {
    let present = runtime.dirs.list_files(root);
    let mut records = Vec::new();
    for source in log_records(runtime, root) {
        let text = runtime.files.read_text(&join(&[root, &source])).map_err(CritiqueError::from_fs)?;
        // A record that is proposed, rejected, superseded or deprecated is no decision the code can be checked against.
        if in_force(&text) {
            records.push(source);
        }
    }
    let glossary = GLOSSARIES.iter().find(|name| present.iter().any(|file| file == *name));
    let maps: Vec<&String> = present.iter().filter(|path| *path == CONTEXT_MAP).collect();
    let references: Vec<String> = glossary.map(|name| name.to_string()).into_iter().chain(maps.iter().map(|path| path.to_string())).chain(records.iter().cloned()).collect();
    let mut documents = Vec::new();
    for source in &references {
        let text = runtime.files.read_text(&join(&[root, source])).map_err(CritiqueError::from_fs)?;
        documents.push(UnfrozenFile::new("reference", &join(&["references", source]), source, &text));
    }
    Ok(DecisionReferences { adrs: records.len(), documents, maps: maps.len() })
}

/// The packet cap less what history may add, so history never takes the packet past the cap.
fn limited(request: &RecoveryRequest) -> PacketLimits {
    let mut limits = request.limits.clone();
    if request.history.is_some() {
        limits.cap_bytes = (limits.cap_bytes - HISTORY_RESERVE_BYTES as f64).max(1.0);
    }
    limits
}

pub fn read_recovery_packet(runtime: &Runtime, root: &str, request: &RecoveryRequest) -> Result<Packet<RepositoryPacket>, CritiqueError> {
    let DecisionReferences { adrs, documents, maps } = decision_references(runtime, root)?;
    let text = documents.iter().map(|file| file.text.as_str()).collect::<Vec<_>>().join("\n\n");
    let packet = DocumentPacket { artifact_text: &text, files: &documents };
    let limits = limited(request);
    let sampled = match sample_tree(runtime, root, &packet, request.head, &limits, Some(source_sample), true)? {
        Packet::Refused(refusal) => return Ok(Packet::Refused(refusal)),
        Packet::Read(sampled) => sampled,
    };
    let history = match request.history {
        None => Vec::new(),
        Some(tracker) => history(runtime, root, &sampled, tracker)?,
    };
    let files: Vec<UnfrozenFile> = sampled.files.iter().cloned().chain(history).collect();
    let repository = PreparedRepository { packet_bytes: files.iter().map(|file| file.text.len()).sum(), ..sampled.repository.clone() };
    Ok(Packet::Read(RepositoryPacket {
        adrs: Some(adrs),
        artifact_text: text,
        context_maps: Some(maps),
        files,
        reference: RECOVERY_ARTIFACT.to_string(),
        repository,
        source_lines: Some(source_lines(&sampled.tracked)),
        steps: None,
        stored_data_steps: None,
    }))
}

/// An issue a commit message cites, or nothing when the tracker cannot read it: a number that is no issue, or a
/// tracker of local files.
fn issue(runtime: &Runtime, tracker: &Tracker, number: f64) -> Option<crate::tracker_resolver::TrackedArtifact> {
    match tracker.artifact(runtime, &issue_reference(number), false) {
        Ok(ArtifactRead::Read(artifact)) => Some(artifact),
        _ => None,
    }
}

/// An issue's text cut to its byte cap, as `Buffer.subarray(0, n).toString('utf8')` cuts it.
fn cut(text: &str, bytes: usize) -> String {
    String::from_utf8_lossy(&text.as_bytes()[..text.len().min(bytes)]).into_owned()
}

/// The commit log of the sampled files, and the issues it cites.
fn history(runtime: &Runtime, root: &str, sampled: &SampledTree, tracker: Option<&Tracker>) -> Result<Vec<UnfrozenFile>, CritiqueError> {
    let paths: Vec<String> = sampled.files.iter().filter(|file| file.kind == "tree").map(|file| file.source.clone()).take(HISTORY_MAX_PATHS).collect();
    let commit = &sampled.repository.head_commit;
    let entries = runtime.git.commit_log(root, commit, &paths, HISTORY_MAX_COMMITS).map_err(CritiqueError::unexpected)?;
    let log = UnfrozenFile::new("history", HISTORY_LOG_PATH, &format!("git log {commit}"), &render_history_log(&entries, commit).0);
    let mut files = vec![log];
    let Some(tracker) = tracker else { return Ok(files) };
    for number in cited_issues(&entries) {
        if let Some(issue) = issue(runtime, tracker, number) {
            files.push(UnfrozenFile::new("history", &join(&[HISTORY_ISSUES_DIR, &issue.name]), &issue.reference, &cut(&issue.text, HISTORY_ISSUE_BYTES)));
        }
    }
    Ok(files)
}
