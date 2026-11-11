//! The proof record: what a headless proof run saw, and how it is stored.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

/// The verdict of a headless proof run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every scenario exited as expected.
    Passed,
    /// At least one scenario did not exit as expected.
    Failed,
    /// The run could not be attempted, so nothing was proved.
    Skipped,
}

impl Outcome {
    /// The record spelling of this outcome.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::Skipped => "skipped",
        }
    }

    /// Parses a record spelling, or `None` when it is not one of the three.
    pub fn parse(s: &str) -> Option<Outcome> {
        match s {
            "passed" => Some(Outcome::Passed),
            "failed" => Some(Outcome::Failed),
            "skipped" => Some(Outcome::Skipped),
            _ => None,
        }
    }
}

/// What one scenario of a proof run did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioResult {
    /// The scenario name from the workflow.
    pub name: String,
    /// The exit code the scenario is expected to produce.
    pub expected_exit: i32,
    /// The exit code gold-band produced, or `None` when it never ran or was killed.
    pub exit_code: Option<i32>,
    /// The run state gold-band printed, such as `completed`.
    pub run_status: Option<String>,
    /// The run outcome gold-band printed, such as `success`.
    pub run_outcome: Option<String>,
    /// Free text, such as why the scenario failed.
    pub note: Option<String>,
}

impl ScenarioResult {
    /// Whether the scenario exited as expected.
    pub fn passed(&self) -> bool {
        self.exit_code == Some(self.expected_exit)
    }
}

/// One recorded headless proof of one workflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// The workflow id the proof belongs to.
    pub workflow_id: String,
    /// The SHA-256 of the workflow file the proof was recorded against.
    pub workflow_sha256: String,
    /// The Gold Band commit the proof ran on, when it was known.
    pub gold_band_commit: Option<String>,
    /// The verdict of the proof run.
    pub outcome: Outcome,
    /// One entry per scenario the proof ran.
    pub scenarios: Vec<ScenarioResult>,
    /// Why the proof was skipped, when it was.
    pub skip_reason: Option<String>,
    /// When the proof was recorded, as RFC 3339 UTC.
    pub recorded_at: String,
}

impl Record {
    /// The record as pretty JSON, with a trailing newline.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        out.push_str(&format!(
            "  \"workflow_id\": {},\n",
            json_string(&self.workflow_id)
        ));
        out.push_str(&format!(
            "  \"workflow_sha256\": {},\n",
            json_string(&self.workflow_sha256)
        ));
        out.push_str(&format!(
            "  \"gold_band_commit\": {},\n",
            json_optional_string(&self.gold_band_commit)
        ));
        out.push_str(&format!(
            "  \"outcome\": {},\n",
            json_string(self.outcome.as_str())
        ));
        if self.scenarios.is_empty() {
            out.push_str("  \"scenarios\": [],\n");
        } else {
            out.push_str("  \"scenarios\": [\n");
            for (index, scenario) in self.scenarios.iter().enumerate() {
                out.push_str("    {\n");
                out.push_str(&format!(
                    "      \"name\": {},\n",
                    json_string(&scenario.name)
                ));
                out.push_str(&format!(
                    "      \"expected_exit\": {},\n",
                    scenario.expected_exit
                ));
                out.push_str(&format!(
                    "      \"exit_code\": {},\n",
                    json_optional_int(scenario.exit_code)
                ));
                out.push_str(&format!(
                    "      \"run_status\": {},\n",
                    json_optional_string(&scenario.run_status)
                ));
                out.push_str(&format!(
                    "      \"run_outcome\": {},\n",
                    json_optional_string(&scenario.run_outcome)
                ));
                out.push_str(&format!(
                    "      \"note\": {}\n",
                    json_optional_string(&scenario.note)
                ));
                let comma = if index + 1 == self.scenarios.len() {
                    ""
                } else {
                    ","
                };
                out.push_str(&format!("    }}{}\n", comma));
            }
            out.push_str("  ],\n");
        }
        out.push_str(&format!(
            "  \"skip_reason\": {},\n",
            json_optional_string(&self.skip_reason)
        ));
        out.push_str(&format!(
            "  \"recorded_at\": {}\n",
            json_string(&self.recorded_at)
        ));
        out.push_str("}\n");
        out
    }

    /// Parses a record, naming the field that is missing or mistyped.
    pub fn from_json(text: &str) -> Result<Record, String> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("the proof record is not valid JSON: {}", error))?;
        let object = value
            .as_object()
            .ok_or_else(|| "the proof record is not a JSON object".to_string())?;
        let outcome_text = string_field(object, "outcome")?;
        let outcome = Outcome::parse(&outcome_text)
            .ok_or_else(|| format!("the proof record has an unknown outcome `{}`", outcome_text))?;
        Ok(Record {
            workflow_id: string_field(object, "workflow_id")?,
            workflow_sha256: string_field(object, "workflow_sha256")?,
            gold_band_commit: optional_string_field(object, "gold_band_commit")?,
            outcome,
            scenarios: scenarios_field(object)?,
            skip_reason: optional_string_field(object, "skip_reason")?,
            recorded_at: string_field(object, "recorded_at")?,
        })
    }
}

/// The path of the proof record of `workflow_id` inside `dir`.
pub fn record_path(dir: &Path, workflow_id: &str) -> PathBuf {
    let safe: String = workflow_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{}.proof.json", safe))
}

/// Writes the record into `dir`, replacing any earlier one, and returns its path.
pub fn write_record(dir: &Path, record: &Record) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|error| {
        format!(
            "cannot create the proof directory {}: {}",
            dir.display(),
            error
        )
    })?;
    let path = record_path(dir, &record.workflow_id);
    let temp = temp_path(dir, &record.workflow_id);
    let written = fs::write(&temp, record.to_json()).and_then(|()| fs::rename(&temp, &path));
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(format!(
            "cannot write the proof record {}: {}",
            path.display(),
            error
        ));
    }
    Ok(path)
}

/// Reads the record of `workflow_id` from `dir`, or `None` when there is none.
pub fn read_record(dir: &Path, workflow_id: &str) -> Result<Option<Record>, String> {
    let path = record_path(dir, workflow_id);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot read the proof record {}: {}",
                path.display(),
                error
            ))
        }
    };
    Record::from_json(&text).map(Some).map_err(|message| {
        format!(
            "cannot parse the proof record {}: {}",
            path.display(),
            message
        )
    })
}

/// The RFC 3339 UTC timestamp of `unix_seconds`, such as `1970-01-01T00:00:00Z`.
pub fn timestamp_utc(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let seconds = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

/// The RFC 3339 UTC timestamp of now, or of the epoch when the clock is before it.
pub fn now_utc() -> String {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH);
    timestamp_utc(since_epoch.map(|elapsed| elapsed.as_secs()).unwrap_or(0))
}

/// The civil date of a count of days since 1970-01-01, by the era algorithm.
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// A unique temporary file name inside `dir`, so a write is never half seen.
fn temp_path(dir: &Path, workflow_id: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let tick = COUNTER.fetch_add(1, Ordering::SeqCst);
    let name = record_path(dir, workflow_id);
    let stem = name
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    dir.join(format!(".{}.{}.{}.tmp", stem, std::process::id(), tick))
}

/// A JSON string literal for `text`.
fn json_string(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

/// A JSON string literal or `null`.
fn json_optional_string(value: &Option<String>) -> String {
    match value {
        Some(text) => json_string(text),
        None => "null".to_string(),
    }
}

/// A JSON integer or `null`.
fn json_optional_int(value: Option<i32>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => "null".to_string(),
    }
}

/// A required string field of the record object.
fn string_field(object: &Map<String, Value>, name: &str) -> Result<String, String> {
    match object.get(name) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(format!("the proof record field `{}` is not a string", name)),
        None => Err(format!("the proof record is missing the field `{}`", name)),
    }
}

/// An optional string field of the record object, absent or `null` meaning none.
fn optional_string_field(
    object: &Map<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!(
            "the proof record field `{}` is not a string or null",
            name
        )),
    }
}

/// A required integer field of the record object.
fn int_field(object: &Map<String, Value>, name: &str) -> Result<i32, String> {
    match object.get(name) {
        Some(Value::Number(number)) => match number.as_i64() {
            Some(value) => i32::try_from(value)
                .map_err(|_| format!("the proof record field `{}` is out of range", name)),
            None => Err(format!(
                "the proof record field `{}` is not an integer",
                name
            )),
        },
        Some(_) => Err(format!(
            "the proof record field `{}` is not an integer",
            name
        )),
        None => Err(format!("the proof record is missing the field `{}`", name)),
    }
}

/// An optional integer field of the record object, absent or `null` meaning none.
fn optional_int_field(object: &Map<String, Value>, name: &str) -> Result<Option<i32>, String> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => int_field(object, name).map(Some),
    }
}

/// The scenario list of the record object.
fn scenarios_field(object: &Map<String, Value>) -> Result<Vec<ScenarioResult>, String> {
    let entries = match object.get("scenarios") {
        Some(Value::Array(entries)) => entries,
        Some(_) => {
            return Err("the proof record field `scenarios` is not an array".to_string());
        }
        None => return Err("the proof record is missing the field `scenarios`".to_string()),
    };
    let mut scenarios = Vec::with_capacity(entries.len());
    for entry in entries {
        let scenario = entry.as_object().ok_or_else(|| {
            "the proof record field `scenarios` holds an entry that is not an object".to_string()
        })?;
        scenarios.push(ScenarioResult {
            name: string_field(scenario, "name")?,
            expected_exit: int_field(scenario, "expected_exit")?,
            exit_code: optional_int_field(scenario, "exit_code")?,
            run_status: optional_string_field(scenario, "run_status")?,
            run_outcome: optional_string_field(scenario, "run_outcome")?,
            note: optional_string_field(scenario, "note")?,
        });
    }
    Ok(scenarios)
}
