//! Why a launched run failed, in words a reader can act on: the kind of failure and one line of reason, decoded
//! from how the client's process ended, what it wrote to stderr and the error its own output carries. A run's
//! `reason` slug stays what the retry and the climb read; this is what the result and the event log add to it.

use crate::adapters::process::signal_name;
use crate::json::{self, Value};
use crate::ports::{Ending, ProcessResult};

/// How much of the end of stderr a result and the event log keep.
pub const TAIL_BYTES: usize = 4_096;

/// How much of the end of stdout a failure keeps when stderr is empty.
pub const OUTPUT_TAIL_BYTES: usize = 2_048;

/// What kind of failure ended a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The client never started: no launcher, a confinement that cannot be applied, a program that is not there.
    SpawnFailed,
    /// A signal ended the client, or the gateway was stopped and stopped it.
    Signalled,
    /// The client outlived its time limit and was stopped.
    TimedOut,
    /// The client's stderr says the sandbox refused it something.
    SandboxDenied,
    /// The client ran and ended badly by its own account: a non-zero exit, an error in its output, no answer.
    HarnessError,
    /// The gateway itself stopped before the run's result was written (an error of its own, or it was killed).
    GatewayError,
    /// The owner's `cancel` or the stall policy ended the run.
    Cancelled,
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureKind::SpawnFailed => "spawn-failed",
            FailureKind::Signalled => "signalled",
            FailureKind::TimedOut => "timed-out",
            FailureKind::SandboxDenied => "sandbox-denied",
            FailureKind::HarnessError => "harness-error",
            FailureKind::GatewayError => "gateway-error",
            FailureKind::Cancelled => "cancelled",
        }
    }
}

/// A failure's kind, one line of reason, and the end of stdout when stderr held nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub reason: String,
    pub output_tail: Option<String>,
}

impl Failure {
    pub fn to_json(&self) -> Value {
        let mut members = vec![("kind", Value::str(self.kind.as_str())), ("reason", Value::str(&self.reason))];
        members.extend(self.output_tail.as_deref().map(|tail| ("output_tail", Value::str(tail))));
        json::object(members)
    }
}

/// The last `max` bytes of `text`, cut at a character boundary.
pub fn tail(text: &str, max: usize) -> String {
    let mut start = text.len().saturating_sub(max);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// The last line of `text` with anything on it, trimmed.
fn last_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).last()
}

/// The first line of stderr that says the sandbox refused something: Seatbelt's and Landlock's words for it.
fn denial(stderr: &str) -> Option<&str> {
    stderr.lines().map(str::trim).find(|line| line.contains("Operation not permitted") || line.contains("deny("))
}

/// Why the launched run failed, given its `reason` slug, how its process ended and the error its own output
/// carries. The order is the order of what explains most: a client that never started, a stop by the gateway,
/// the time limit, a signal, an error the client reported, a sandbox refusal on stderr, and last whatever
/// stderr ended with, or the exit code alone.
pub fn failure_of(reason: &str, result: &ProcessResult, ending: &Ending, client_error: Option<&str>) -> Failure {
    let (kind, why) = if reason == "launch-failed" {
        (FailureKind::SpawnFailed, last_line(&result.stderr).unwrap_or("the client could not be started").to_string())
    } else if let Some(stopped) = ending.cancelled_by {
        (FailureKind::Signalled, format!("the gateway was stopped by {} and stopped the client's process group", signal_name(stopped)))
    } else if result.timed_out {
        (FailureKind::TimedOut, format!("the client outlived its time limit and was stopped{}", ended_by(ending)))
    } else if let Some(signal) = ending.signal {
        (FailureKind::Signalled, format!("the client was ended by {}{}", signal_name(signal), stderr_said(&result.stderr)))
    } else if let Some(error) = client_error.filter(|error| !error.trim().is_empty()) {
        (FailureKind::HarnessError, format!("the client reported: {}", error.trim()))
    } else if let Some(line) = denial(&result.stderr) {
        (FailureKind::SandboxDenied, format!("{line} (the sandbox's own denial log is not captured)"))
    } else if let Some(line) = last_line(&result.stderr) {
        (FailureKind::HarnessError, line.to_string())
    } else {
        (FailureKind::HarnessError, format!("{reason}: the client exited {} with nothing on stderr and no error in its output", result.exit_code))
    };
    let why = match kind {
        FailureKind::HarnessError if cut_by_cloudflare(&[client_error.unwrap_or(""), &result.stderr, &result.stdout]) => format!("{why}{CLOUDFLARE_HINT}"),
        _ => why,
    };
    let output_tail = (result.stderr.trim().is_empty() && !result.stdout.trim().is_empty()).then(|| tail(&result.stdout, OUTPUT_TAIL_BYTES));
    Failure { kind, reason: why, output_tail }
}

/// What a Cloudflare 524 from the model proxy means for the client that met it.
const CLOUDFLARE_HINT: &str = " (the model proxy sits behind Cloudflare, which cuts a request that is not streamed at about 120 s with a 524 timeout: this client probably did not send `stream: true`; every LiteLLM client must stream)";

/// Whether any of the texts reports a Cloudflare 524 timeout: the status with the word `timeout` or `cloudflare`.
fn cut_by_cloudflare(texts: &[&str]) -> bool {
    texts.iter().map(|text| text.to_ascii_lowercase()).any(|text| text.contains("524") && (text.contains("timeout") || text.contains("timed out") || text.contains("cloudflare")))
}

/// `; ended by SIGKILL` when a signal ended the process, else nothing.
fn ended_by(ending: &Ending) -> String {
    ending.signal.map(|signal| format!("; ended by {}", signal_name(signal))).unwrap_or_default()
}

/// `; stderr: <its last line>` when stderr has one, else nothing.
fn stderr_said(stderr: &str) -> String {
    last_line(stderr).map(|line| format!("; stderr: {line}")).unwrap_or_default()
}
