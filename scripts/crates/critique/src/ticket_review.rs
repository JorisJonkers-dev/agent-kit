//! Records what a ticket review decided, for a commit gate to read without a model call. A review fails while an
//! accepted critical finding is unfixed, and is degraded when no critic reported. A review of the staged change also
//! hashes the index as it is at record time, after the fixes and the kept regression tests were staged: that is the
//! change the review vouches for. The node runner's `application/ticket-review.ts` and `domain/ticket-review.ts`.

use crate::adjudication::AdjudicatedRun;
use crate::error::CritiqueError;
use crate::manifest::{spec_reference, RunManifest};
use crate::paths::join;
use crate::runtime::Runtime;
use crate::staged_change::read_staged;
use crate::verdicts::unfixed_critical;
use model_gateway::json::{self, Value};
use model_gateway::text::dirname;
use model_gateway::timestamps::to_iso;

/// The review record a ticket-review run holds, in the run directory.
pub fn ticket_review_path(run_dir: &str) -> String {
    join(&[run_dir, "review.json"])
}

/// Every recorded ticket review of the repository, one JSON line per run in the critique directory of the git common
/// directory, oldest first: the last line whose `staged_diff` matches the change about to be committed is that change's
/// review.
pub fn ticket_review_index_path(critique_dir: &str) -> String {
    join(&[critique_dir, "ticket-reviews.jsonl"])
}

/// A critic that did not report: its id, and its failure (or its status, for a critic that was skipped or discarded).
#[derive(Clone, Debug, PartialEq)]
pub struct CriticFailureLine {
    pub critic_id: String,
    pub reason: String,
}

/// A recorded ticket review, written so a commit gate can read it without any model call.
#[derive(Clone, Debug, PartialEq)]
pub struct TicketReviewRecord {
    pub run_id: String,
    pub recorded_at: String,
    pub repository: String,
    /// The spec's reference, as the packet's spec file names it.
    pub spec: String,
    /// The ticket's reference, as the manifest's artifact names it.
    pub ticket: String,
    pub base_commit: String,
    /// The commit the critics read: for a staged review, a commit made of the index.
    pub head_commit: String,
    /// The hash of the staged change the critics read; null when the review was of a commit.
    pub reviewed_staged_diff: Option<String>,
    /// The hash of the staged change when the review was recorded, after the accepted findings were fixed: the change
    /// this review vouches for. Null when the review was of a commit.
    pub staged_diff: Option<String>,
    /// `fail` while an accepted critical finding is unfixed; `degraded` when no critic reported; otherwise `pass`.
    pub review: String,
    /// True when no critic produced a usable review.
    pub degraded: bool,
    /// Every critic of the run that did not report, and why; empty when all reported.
    pub critic_failures: Vec<CriticFailureLine>,
    /// The accepted critical findings not yet fixed.
    pub unfixed_critical: Vec<String>,
}

impl TicketReviewRecord {
    /// The record as `review.json` and the review index hold it: `JSON.stringify` order.
    pub fn to_json(&self) -> Value {
        let text_or_null = |text: &Option<String>| text.as_ref().map_or(Value::Null, |text| Value::str(text));
        let failures = self
            .critic_failures
            .iter()
            .map(|failure| json::object(vec![("critic_id", Value::str(&failure.critic_id)), ("reason", Value::str(&failure.reason))]))
            .collect();
        json::object(vec![
            ("base_commit", Value::str(&self.base_commit)),
            ("critic_failures", Value::Arr(failures)),
            ("degraded", Value::Bool(self.degraded)),
            ("head_commit", Value::str(&self.head_commit)),
            ("recorded_at", Value::str(&self.recorded_at)),
            ("repository", Value::str(&self.repository)),
            ("review", Value::str(&self.review)),
            ("reviewed_staged_diff", text_or_null(&self.reviewed_staged_diff)),
            ("run_id", Value::str(&self.run_id)),
            ("spec", Value::str(&self.spec)),
            ("stage", Value::str("ticket-review")),
            ("staged_diff", text_or_null(&self.staged_diff)),
            ("ticket", Value::str(&self.ticket)),
            ("unfixed_critical", Value::Arr(self.unfixed_critical.iter().map(|id| Value::str(id)).collect())),
            ("version", Value::Num(1.0)),
        ])
    }
}

/// The record of a ticket review whose verdicts are adjudicated.
pub fn compute_ticket_review(runtime: &Runtime, manifest: &RunManifest, run: &AdjudicatedRun) -> Result<TicketReviewRecord, CritiqueError> {
    let Some(code) = &manifest.code else {
        return Err(CritiqueError::new("invalid-run", format!("run {} is a ticket review without the commits it reviewed", manifest.run_id)));
    };
    let reviewed = code.staged_diff.clone();
    let unfixed: Vec<String> = unfixed_critical(&run.stage, &run.findings).iter().map(|f| f.finding.id.clone()).collect();
    let critic_failures = run
        .critics
        .iter()
        .filter(|critic| critic.status != "ok")
        .map(|critic| CriticFailureLine { critic_id: critic.critic_id.clone(), reason: critic.failure.clone().unwrap_or_else(|| critic.status.clone()) })
        .collect();
    let staged_diff = match reviewed {
        Some(_) => Some(read_staged(runtime, &manifest.repo_root)?.sha256),
        None => None,
    };
    Ok(TicketReviewRecord {
        base_commit: code.base_commit.clone(),
        critic_failures,
        degraded: run.degraded,
        head_commit: code.head_commit.clone(),
        recorded_at: to_iso(runtime.clock.now_ms()),
        repository: manifest.repository.clone(),
        review: if !unfixed.is_empty() { "fail" } else if run.degraded { "degraded" } else { "pass" }.to_string(),
        reviewed_staged_diff: code.staged_diff.clone(),
        run_id: manifest.run_id.clone(),
        spec: spec_reference(manifest),
        staged_diff,
        ticket: manifest.artifact.reference.clone(),
        unfixed_critical: unfixed,
    })
}

/// Writes the record into the run and appends it to the repository's review index.
pub fn save_ticket_review(runtime: &Runtime, manifest: &RunManifest, record: &TicketReviewRecord) -> Result<(), CritiqueError> {
    let value = record.to_json();
    runtime.files.write_text(&ticket_review_path(&manifest.run_dir), &format!("{}\n", json::pretty(&value))).map_err(CritiqueError::from_fs)?;
    let index = ticket_review_index_path(dirname(&manifest.run_dir));
    runtime.files.mkdir_all(dirname(&index)).map_err(CritiqueError::from_fs)?;
    runtime.files.append(&index, &format!("{}\n", json::compact(&value))).map_err(CritiqueError::from_fs)
}
