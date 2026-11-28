//! A run on a free model whose every free copy is briefly limited fails with a refusal that names when the first
//! copy is back (`... the first is back in 5s`). A short wait is cheaper than a fall back to another route, so the
//! gateway waits the named time (plus a small margin) and runs the same model again, a bounded number of times and
//! up to a total wait cap (`settings.retry_wait`). A wait longer than what is left of the cap is not taken: the
//! run goes on to the free fallback (`free_fallback.rs`) as it did before. Every retry is a `retry` event of the
//! run that retries and a `retry` member of its result.
//!
//! The wait is the gateway's, not the proxy's: ADR-0018 keeps the proxy from holding a request.

use crate::error::GatewayError;
use crate::free_fallback::available_after;
use crate::json::{self, Value};

/// What `settings.retry_wait` says: how many times a run is retried, the most it waits in all, and the margin
/// added to every wait named.
#[derive(Clone, Debug, PartialEq)]
pub struct RetryWaitSettings {
    pub max_retries: u32,
    pub max_total_wait_s: f64,
    pub margin_s: f64,
}

impl Default for RetryWaitSettings {
    fn default() -> Self {
        RetryWaitSettings { max_retries: 3, max_total_wait_s: 60.0, margin_s: 1.0 }
    }
}

const FIELDS: [&str; 3] = ["max_retries", "max_total_wait_s", "margin_s"];

/// The most retries or seconds a setting may name (a day).
const MAX: f64 = 86_400.0;

fn invalid(message: String) -> GatewayError {
    GatewayError::new("invalid-config", message)
}

/// `base` with what `raw` (a `settings.retry_wait` object) states over it; zero retries or zero seconds turn the
/// wait off.
pub fn merge(base: &RetryWaitSettings, raw: Option<&Value>) -> Result<RetryWaitSettings, GatewayError> {
    let Some(raw) = raw else { return Ok(base.clone()) };
    let Some(object) = raw.as_object() else { return Err(invalid("settings.retry_wait must be an object".to_string())) };
    if let Some((name, _)) = object.entries().into_iter().find(|(name, _)| !FIELDS.contains(name)) {
        return Err(invalid(format!("settings.retry_wait.{name} is not a setting: it has {}", FIELDS.join(", "))));
    }
    let number = |name: &str, current: f64| match object.get(name) {
        None => Ok(current),
        Some(Value::Num(value)) if (0.0..=MAX).contains(value) => Ok(*value),
        Some(_) => Err(invalid(format!("settings.retry_wait.{name} must be a number from 0 to {MAX}"))),
    };
    let retries = number("max_retries", f64::from(base.max_retries))?;
    if retries.fract() != 0.0 {
        return Err(invalid("settings.retry_wait.max_retries must be a whole number".to_string()));
    }
    Ok(RetryWaitSettings { max_retries: retries as u32, max_total_wait_s: number("max_total_wait_s", base.max_total_wait_s)?, margin_s: number("margin_s", base.margin_s)? })
}

/// One retry: the run it follows, why, and what was waited.
#[derive(Clone, Debug, PartialEq)]
pub struct RetryReport {
    /// The number of this retry, from 1.
    pub retry: u32,
    /// The run id of the run that was refused.
    pub of_run_id: String,
    /// The refusal the run ended with, as its client printed it.
    pub reason: String,
    /// The seconds the refusal named.
    pub back_in_s: i64,
    /// The seconds waited before this run: what the refusal named plus the margin.
    pub waited_s: f64,
    /// The seconds waited over every retry so far, this one included.
    pub total_waited_s: f64,
    pub max_retries: u32,
    pub max_total_wait_s: f64,
}

impl RetryReport {
    /// The `retry` object of a result and of the run's event.
    pub fn to_json(&self) -> Value {
        json::object(vec![
            ("back_in_s", Value::Num(self.back_in_s as f64)),
            ("max_retries", Value::Num(f64::from(self.max_retries))),
            ("max_total_wait_s", Value::Num(self.max_total_wait_s)),
            ("of_run_id", Value::str(&self.of_run_id)),
            ("reason", Value::str(&self.reason)),
            ("retry", Value::Num(f64::from(self.retry))),
            ("total_waited_s", Value::Num(self.total_waited_s)),
            ("waited_s", Value::Num(self.waited_s)),
        ])
    }

    /// The line the summary opens with.
    pub fn notice(&self) -> String {
        format!("retry {} of at most {} after waiting {}s for the free copies the proxy said were back in {}s", self.retry, self.max_retries, self.waited_s, self.back_in_s)
    }
}

/// The waits of one attempt's retries: how many it took and how long they were.
#[derive(Debug)]
pub struct Waiter {
    settings: RetryWaitSettings,
    retries: u32,
    waited_s: f64,
}

/// What a refusal asks of the run: the milliseconds to sleep, and the report of the retry that follows.
pub struct Wait {
    pub sleep_ms: u64,
    pub report: RetryReport,
}

impl Waiter {
    pub fn new(settings: RetryWaitSettings) -> Waiter {
        Waiter { settings, retries: 0, waited_s: 0.0 }
    }

    /// The wait the refusal asks for, when it names a time the run may wait for: the refusal names how long until
    /// a copy is back, the retries are not used up, and the wait still fits the total cap. `None` leaves the run
    /// to its other ways out, the free fallback first.
    pub fn next(&mut self, run_id: &str, refusal: &str, now_ms: i64) -> Option<Wait> {
        let back_in_s = available_after(refusal, now_ms).0?;
        let wait_s = back_in_s as f64 + self.settings.margin_s;
        if self.retries >= self.settings.max_retries || self.waited_s + wait_s > self.settings.max_total_wait_s {
            return None;
        }
        self.retries += 1;
        self.waited_s += wait_s;
        let report = RetryReport {
            retry: self.retries,
            of_run_id: run_id.to_string(),
            reason: refusal.to_string(),
            back_in_s,
            waited_s: wait_s,
            total_waited_s: self.waited_s,
            max_retries: self.settings.max_retries,
            max_total_wait_s: self.settings.max_total_wait_s,
        };
        Some(Wait { sleep_ms: (wait_s * 1000.0).round() as u64, report })
    }
}
