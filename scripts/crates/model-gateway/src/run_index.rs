//! The global run index: `run-index.jsonl` in the state directory, one JSON line per run start, finish and
//! removal, so the status commands (`ps`, `status`, `cancel`, `gc`) find a run whichever repository it ran in
//! without scanning run directories. The run's own `events.jsonl` stays the record of what happened; the index
//! says where it is and, once the run ended, how.
//!
//! Every line is one append, so any number of gateways write the index at once without a lock. A file that
//! reaches `ROTATE_BYTES` is renamed to `run-index.<ms>-<random>.jsonl` and a fresh one starts; an append that
//! races the rename lands in the renamed file, so no line is lost. Only the newest `KEEP_ROTATED` rotated
//! files stay. A line that does not parse is skipped.

use crate::json::{self, Value};
use crate::ports::{Clock, Files};
use crate::text::{dirname, join};
use crate::timestamps::{parse, to_iso};
use std::collections::HashMap;

/// The index's file name in the state directory.
pub const INDEX_FILE: &str = "run-index.jsonl";

/// How large the index may grow before it is rotated.
pub const ROTATE_BYTES: u64 = 1_048_576;

/// How many rotated files are kept beside the index.
pub const KEEP_ROTATED: usize = 8;

/// The version every index line carries.
pub const INDEX_VERSION: f64 = 1.0;

/// What one index line records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexKind {
    Started,
    Finished,
    Removed,
}

impl IndexKind {
    fn as_str(self) -> &'static str {
        match self {
            IndexKind::Started => "started",
            IndexKind::Finished => "finished",
            IndexKind::Removed => "removed",
        }
    }

    fn of(text: &str) -> Option<IndexKind> {
        [IndexKind::Started, IndexKind::Finished, IndexKind::Removed].into_iter().find(|kind| kind.as_str() == text)
    }
}

/// One line of the index.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexEntry {
    pub kind: IndexKind,
    pub ts_ms: i64,
    pub run_id: String,
    pub run_dir: String,
    pub workflow: String,
    pub client: String,
    pub model: String,
    pub family: String,
    pub mode: String,
    /// The gateway process that ran it; set on a start.
    pub gateway_pid: Option<u32>,
    /// How a finished run ended: `ok`, `failed`, `cancelled` or `lost`.
    pub status: Option<String>,
    pub reason: Option<String>,
}

impl IndexEntry {
    fn to_json(&self) -> Value {
        let mut members = vec![
            ("v", Value::Num(INDEX_VERSION)),
            ("ts", Value::str(&to_iso(self.ts_ms))),
            ("kind", Value::str(self.kind.as_str())),
            ("run_id", Value::str(&self.run_id)),
            ("run_dir", Value::str(&self.run_dir)),
            ("workflow", Value::str(&self.workflow)),
            ("client", Value::str(&self.client)),
            ("model", Value::str(&self.model)),
            ("family", Value::str(&self.family)),
            ("mode", Value::str(&self.mode)),
        ];
        members.extend(self.gateway_pid.map(|pid| ("gateway_pid", Value::Num(f64::from(pid)))));
        members.extend(self.status.as_deref().map(|status| ("status", Value::str(status))));
        members.extend(self.reason.as_deref().map(|reason| ("reason", Value::str(reason))));
        json::object(members)
    }

    fn from_json(line: &str) -> Option<IndexEntry> {
        let value = json::parse(line).ok()?;
        let object = value.as_object()?;
        let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_string);
        Some(IndexEntry {
            kind: IndexKind::of(&text("kind")?)?,
            ts_ms: parse(&text("ts")?)?,
            run_id: text("run_id")?,
            run_dir: text("run_dir")?,
            workflow: text("workflow").unwrap_or_default(),
            client: text("client").unwrap_or_default(),
            model: text("model").unwrap_or_default(),
            family: text("family").unwrap_or_default(),
            mode: text("mode").unwrap_or_default(),
            gateway_pid: match object.get("gateway_pid") {
                Some(Value::Num(pid)) => Some(*pid as u32),
                _ => None,
            },
            status: text("status"),
            reason: text("reason"),
        })
    }
}

/// The index file and its rotated copies.
pub struct RunIndex<'a> {
    pub files: &'a dyn Files,
    pub clock: &'a dyn Clock,
    pub path: String,
    /// The size at which the index is rotated; `ROTATE_BYTES` unless a test sets another.
    pub rotate_bytes: u64,
    /// How many rotated files stay; `KEEP_ROTATED` unless a test sets another.
    pub keep: usize,
}

impl<'a> RunIndex<'a> {
    pub fn new(files: &'a dyn Files, clock: &'a dyn Clock, path: &str) -> RunIndex<'a> {
        RunIndex { files, clock, path: path.to_string(), rotate_bytes: ROTATE_BYTES, keep: KEEP_ROTATED }
    }

    /// Adds one line, rotating the index first when it is full. The index never fails a run: a line that cannot
    /// be written is dropped.
    pub fn record(&self, entry: &IndexEntry) {
        let _ = self.files.mkdir_all(dirname(&self.path));
        self.rotate_if_full();
        let _ = self.files.append(&self.path, &format!("{}\n", json::compact(&entry.to_json())));
    }

    fn base(&self) -> &str {
        crate::text::basename(&self.path)
    }

    /// The names of the rotated files beside the index, oldest first.
    fn rotated(&self) -> Vec<String> {
        let prefix = format!("{}.", self.base().trim_end_matches(".jsonl"));
        let mut names: Vec<String> = self.files.list_dir(dirname(&self.path)).unwrap_or_default().into_iter().filter(|name| name.starts_with(&prefix) && name.ends_with(".jsonl") && name != self.base()).collect();
        names.sort();
        names
    }

    fn rotate_if_full(&self) {
        if self.files.size(&self.path).unwrap_or(0) < self.rotate_bytes {
            return;
        }
        let stem = self.base().trim_end_matches(".jsonl");
        let to = join(&[dirname(&self.path), &format!("{stem}.{:013}-{}.jsonl", self.clock.now_ms(), crate::commands::random_suffix())]);
        // A gateway that rotated first has already moved the file: the rename then fails and nothing is lost.
        if self.files.rename(&self.path, &to).is_err() {
            return;
        }
        let rotated = self.rotated();
        for name in rotated.iter().take(rotated.len().saturating_sub(self.keep)) {
            let _ = self.files.remove_file(&join(&[dirname(&self.path), name]));
        }
    }

    /// Every line of every file, oldest first.
    pub fn read(&self) -> Vec<IndexEntry> {
        let mut paths: Vec<String> = self.rotated().iter().map(|name| join(&[dirname(&self.path), name])).collect();
        paths.push(self.path.clone());
        paths.iter().filter_map(|path| self.files.read_text(path).ok()).flat_map(|text| text.lines().filter_map(IndexEntry::from_json).collect::<Vec<_>>()).collect()
    }

    /// The rotated files with the moment each was last written, for the retention to age out.
    pub fn rotated_files(&self) -> Vec<(String, Option<i64>)> {
        self.rotated().into_iter().map(|name| (join(&[dirname(&self.path), &name]), self.files.modified_ms(&join(&[dirname(&self.path), &name])))).collect()
    }
}

/// A run as the index describes it after its lines are folded together.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexedRun {
    pub run_id: String,
    pub run_dir: String,
    pub workflow: String,
    pub client: String,
    pub model: String,
    pub family: String,
    pub mode: String,
    pub started_ms: i64,
    pub gateway_pid: Option<u32>,
    /// How it ended, when the index says it did: status, reason and the moment.
    pub finished: Option<(String, Option<String>, i64)>,
}

/// The runs the entries describe, in the order they started; a run whose directory was removed is left out.
pub fn fold(entries: &[IndexEntry]) -> Vec<IndexedRun> {
    let mut runs: Vec<IndexedRun> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for entry in entries.iter().filter(|entry| entry.kind == IndexKind::Started) {
        if at.contains_key(&entry.run_id) {
            continue;
        }
        at.insert(entry.run_id.clone(), runs.len());
        runs.push(IndexedRun {
            run_id: entry.run_id.clone(),
            run_dir: entry.run_dir.clone(),
            workflow: entry.workflow.clone(),
            client: entry.client.clone(),
            model: entry.model.clone(),
            family: entry.family.clone(),
            mode: entry.mode.clone(),
            started_ms: entry.ts_ms,
            gateway_pid: entry.gateway_pid,
            finished: None,
        });
    }
    let mut removed: Vec<&str> = Vec::new();
    for entry in entries {
        match (entry.kind, at.get(&entry.run_id)) {
            (IndexKind::Finished, Some(index)) if runs[*index].finished.is_none() => {
                runs[*index].finished = Some((entry.status.clone().unwrap_or_default(), entry.reason.clone(), entry.ts_ms));
            }
            (IndexKind::Removed, _) => removed.push(&entry.run_id),
            _ => {}
        }
    }
    runs.retain(|run| !removed.contains(&run.run_id.as_str()));
    runs
}
