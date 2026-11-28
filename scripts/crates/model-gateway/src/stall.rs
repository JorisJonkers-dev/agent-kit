//! The stall policy: a run whose client has written nothing for `threshold_s` seconds gets a `stall` event, and
//! a run still silent at three times that is cancelled. A cancel (this one, or the owner's `cancel`) leaves a
//! marker file in the run directory, `cancel.requested`, whose first line says why; the run's result reads it,
//! so the run ends as `cancelled` and is never retried.

use crate::error::GatewayError;
use crate::json::{Object, Value};
use crate::ports::Files;
use crate::text::join;

/// The marker a cancel leaves in the run directory.
pub const CANCEL_FILE: &str = "cancel.requested";

/// How long a client may be silent before the run is called stalled, unless `settings.stall.threshold_s` says
/// otherwise. A model that thinks for a long time writes nothing: three minutes is past most of those, and the
/// cancel at three times that, nine minutes, is inside a quarter hour's time limit.
pub const DEFAULT_THRESHOLD_S: f64 = 180.0;

/// A run is cancelled when it has been silent this many times the threshold.
pub const CANCEL_FACTOR: u64 = 3;

/// How often the silence is looked at.
pub const CHECK_MS: u64 = 1_000;

/// What `settings.stall` holds.
#[derive(Clone, Debug, PartialEq)]
pub struct StallSettings {
    pub threshold_s: f64,
}

impl Default for StallSettings {
    fn default() -> Self {
        StallSettings { threshold_s: DEFAULT_THRESHOLD_S }
    }
}

fn invalid(message: &str) -> GatewayError {
    GatewayError::new("invalid-config", message.to_string())
}

/// The settings with `settings.stall` of a machine file laid over them.
pub fn merge(base: &StallSettings, raw: Option<&Value>) -> Result<StallSettings, GatewayError> {
    let Some(raw) = raw else { return Ok(base.clone()) };
    let Some(object) = raw.as_object() else { return Err(invalid("settings.stall must be an object")) };
    if let Some((name, _)) = object.entries().into_iter().find(|(name, _)| *name != "threshold_s") {
        return Err(GatewayError::new("invalid-config", format!("settings.stall.{name} is not a setting: it has threshold_s")));
    }
    match object.get("threshold_s") {
        None => Ok(base.clone()),
        Some(Value::Num(seconds)) if seconds.is_finite() && *seconds > 0.0 => Ok(StallSettings { threshold_s: *seconds }),
        Some(_) => Err(invalid("settings.stall.threshold_s must be a number of seconds above 0")),
    }
}

/// What the watch does about a silence of a given length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StallStep {
    Quiet,
    /// Write the `stall` event.
    Warn,
    /// Cancel the run.
    Cancel,
}

/// When a run is stalled and when it is cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StallPolicy {
    pub threshold_ms: u64,
    pub check_ms: u64,
    /// How long a cancelled client has to exit after SIGTERM before its group is killed.
    pub kill_grace_ms: u64,
}

impl StallPolicy {
    pub fn from_settings(settings: &StallSettings) -> StallPolicy {
        StallPolicy { threshold_ms: (settings.threshold_s * 1000.0) as u64, check_ms: CHECK_MS, kill_grace_ms: crate::adapters::process::KILL_GRACE_MS }
    }

    /// The silence at which the run is cancelled.
    pub fn cancel_ms(&self) -> u64 {
        self.threshold_ms * CANCEL_FACTOR
    }

    /// The step for a silence of `silent_ms`, given whether the stall was already written and the run already
    /// cancelled: each is done once.
    pub fn step(&self, silent_ms: u64, warned: bool, cancelled: bool) -> StallStep {
        if silent_ms >= self.cancel_ms() && !cancelled {
            StallStep::Cancel
        } else if silent_ms >= self.threshold_ms && !warned {
            StallStep::Warn
        } else {
            StallStep::Quiet
        }
    }
}

/// Leaves the cancel marker with `why` as its first line; `false` when a cancel already left one.
pub fn mark_cancelled(files: &dyn Files, run_dir: &str, why: &str) -> bool {
    let path = join(&[run_dir, CANCEL_FILE]);
    if files.create_exclusive(&path) != Ok(true) {
        return false;
    }
    let _ = files.append(&path, &format!("{why}\n"));
    true
}

/// Why the run was cancelled, when a marker says it was.
pub fn cancel_reason(files: &dyn Files, run_dir: &str) -> Option<String> {
    let path = join(&[run_dir, CANCEL_FILE]);
    if files.exists(&path) != Ok(true) {
        return None;
    }
    let text = files.read_text(&path).ok()?;
    Some(text.lines().next().unwrap_or_default().trim().to_string())
}

/// A members list for the `stall` event.
pub fn stall_data(silent_s: u64, policy: &StallPolicy) -> Object {
    let mut data = Object::new();
    data.set("silent_s", Value::Num(silent_s as f64));
    data.set("threshold_s", Value::Num((policy.threshold_ms / 1000) as f64));
    data.set("cancel_at_s", Value::Num((policy.cancel_ms() / 1000) as f64));
    data
}
