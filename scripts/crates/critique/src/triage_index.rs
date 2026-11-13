//! Every triage of a staged change in the repository, one JSON line each in the critique directory of the git common
//! directory, oldest first. The commit gate reads the ticket's labels from the last line staged on the current HEAD,
//! so `risk:high` gates a commit without calling the tracker.

use crate::error::CritiqueError;
use crate::paths::join;
use crate::runtime::Runtime;
use model_gateway::json::{compact, Object, Value};
use model_gateway::text::dirname;

/// One triage of a staged change: which ticket, staged on which commit, and the labels the tracker gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct TriageIndexRecord {
    pub run_id: String,
    pub recorded_at: String,
    pub repository: String,
    pub ticket: String,
    pub labels: Vec<String>,
    /// HEAD when the change was triaged: the commit it is staged on.
    pub staged_on: String,
    /// `staged_diff_hash` of the change triaged.
    pub staged_diff: String,
    /// `risky` or `low-risk`.
    pub status: String,
}

impl TriageIndexRecord {
    /// The record as one line holds it: keys in alphabetical order, as the node runner writes them.
    pub fn to_json(&self) -> Value {
        let mut object = Object::new();
        object.set("labels", Value::Arr(self.labels.iter().map(|label| Value::str(label)).collect()));
        object.set("recorded_at", Value::str(&self.recorded_at));
        object.set("repository", Value::str(&self.repository));
        object.set("run_id", Value::str(&self.run_id));
        object.set("staged_diff", Value::str(&self.staged_diff));
        object.set("staged_on", Value::str(&self.staged_on));
        object.set("status", Value::str(&self.status));
        object.set("ticket", Value::str(&self.ticket));
        object.set("version", Value::Num(1.0));
        Value::Obj(object)
    }
}

/// The index file inside a repository's critique directory.
pub fn triage_index_path(critique_dir: &str) -> String {
    join(&[critique_dir, "triages.jsonl"])
}

/// Appends a record to the repository's triage index.
pub fn append_triage_index(runtime: &Runtime, common_dir: &str, record: &TriageIndexRecord) -> Result<(), CritiqueError> {
    let path = triage_index_path(&join(&[common_dir, "critique"]));
    runtime.files.mkdir_all(dirname(&path)).map_err(CritiqueError::from_fs)?;
    runtime.files.append(&path, &format!("{}\n", compact(&record.to_json()))).map_err(CritiqueError::from_fs)
}
