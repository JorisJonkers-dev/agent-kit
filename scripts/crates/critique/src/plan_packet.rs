//! What the plan stage freezes. The node runner's `application/plan-packet.ts`.
//!
//! The plan under `artifact/`, the checker's report on it under `check/plan.json`, the tree sampled at the head the
//! plan targets (the files its anchors and touches name first), `tree/MANIFEST.tsv` and `facts/`, and under
//! `references/` the accepted ADRs and the glossary. The plan is never committed, so its path may lie in the git
//! common directory; no tracker is read.

use crate::adr_packet::{sample_tree, DocumentPacket};
use crate::check_plan::{check_file, report};
use crate::error::CritiqueError;
use crate::packet_files::{Packet, RepositoryPacket, UnfrozenFile};
use crate::plan_file::{relative, PLAN_FILE};
use crate::recovery_packet::decision_references;
use crate::repository::refusal;
use crate::repository_packet::PacketLimits;
use crate::runtime::Runtime;
use model_gateway::json;

/// The packet path of the plan checker's report.
pub const PLAN_CHECK_PATH: &str = "check/plan.json";

pub struct PlanRequest<'a> {
    /// The plan file, absolute.
    pub path: &'a str,
    /// The commit, or branch, the plan targets: its tree is sampled.
    pub head: &'a str,
    /// Routing's packet limits.
    pub limits: &'a PacketLimits,
}

pub fn read_plan_packet(runtime: &Runtime, root: &str, request: &PlanRequest) -> Result<Packet<RepositoryPacket>, CritiqueError> {
    if !runtime.files.exists(request.path).map_err(CritiqueError::from_fs)? {
        return Ok(Packet::Refused(refusal(
            "no-plan",
            &format!("found no refactoring plan at {}; draft {PLAN_FILE} there first, or name the plan file with plan", request.path),
        )));
    }
    let text = runtime.files.read_text(request.path).map_err(CritiqueError::from_fs)?;
    let check = check_file(runtime, request.path, Some(root))?;
    let relative_path = relative(root, request.path);
    let reference = if relative_path.starts_with("..") { request.path.to_string() } else { relative_path };
    let documents = decision_references(runtime, root)?.documents;
    let mut files = vec![
        UnfrozenFile::new("artifact", &format!("artifact/{PLAN_FILE}"), &reference, &text),
        UnfrozenFile::new("check", PLAN_CHECK_PATH, &format!("check-plan {reference}"), &format!("{}\n", json::pretty(&report(&check)))),
    ];
    files.extend(documents);
    let packet = DocumentPacket { artifact_text: &text, files: &files };
    match sample_tree(runtime, root, &packet, request.head, request.limits, None, true)? {
        Packet::Refused(refusal) => Ok(Packet::Refused(refusal)),
        Packet::Read(sampled) => Ok(Packet::Read(RepositoryPacket {
            adrs: None,
            artifact_text: text,
            context_maps: None,
            files: sampled.files,
            reference,
            repository: sampled.repository,
            source_lines: None,
            steps: Some(check.steps.len()),
            stored_data_steps: Some(check.stored_data_steps.len()),
        })),
    }
}
