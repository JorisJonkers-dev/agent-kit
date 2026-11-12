//! Each client's own login status command.
//!
//! Each client's own status command, run read-only through the process port: it
//! says whether the client is signed in, and never sends a prompt. A client whose
//! status command cannot be run, or does not answer in time, is `unknown`, not
//! signed out: silence is not a refusal.

use crate::ports::{ProcessResult, Runtime};
use crate::usage::{ClientStatusReading, SignedIn};

/// How long a status command may take before the gateway stops waiting for it.
pub const STATUS_TIMEOUT_MS: u64 = 5_000;

/// What a status command that outlived its time limit leaves the gateway knowing.
const TIMED_OUT: &str = "status command did not answer in time";

/// What a status command's own answer says, once it has answered at all.
struct StatusVerdict {
    signed_in: SignedIn,
    detail: String,
}

/// What running a status command came to: its answer, or why it could not be run.
enum StatusRun {
    Result(ProcessResult),
    Error(String),
}

/// What a client's own status command says about it; `None` for a client the
/// gateway has no status command for.
pub fn read_status(runtime: &Runtime, id: &str, timeout_ms: u64) -> Option<ClientStatusReading> {
    match id {
        "codex" => Some(reading_of(
            "codex login status",
            run_status(runtime, "codex", &["login", "status"], timeout_ms),
            login_reading,
        )),
        "hermes" => Some(reading_of(
            "hermes auth list",
            run_status(runtime, "hermes", &["auth", "list"], timeout_ms),
            listing_reading,
        )),
        "opencode" => Some(reading_of(
            "opencode auth list",
            run_status(runtime, "opencode", &["auth", "list"], timeout_ms),
            listing_reading,
        )),
        _ => None,
    }
}

fn run_status(runtime: &Runtime, command: &str, args: &[&str], timeout_ms: u64) -> StatusRun {
    match runtime.process.exec(command, args, Some(timeout_ms)) {
        Ok(result) => StatusRun::Result(result),
        Err(error) => StatusRun::Error(error),
    }
}

/// A status command's reading: the two answers it can never give are that it did
/// not run at all, and that it did not answer in time. Both are `unknown`, so a
/// hung or broken client never reads as signed out.
fn reading_of(command: &str, run: StatusRun, decide: fn(&ProcessResult) -> StatusVerdict) -> ClientStatusReading {
    match run {
        StatusRun::Error(error) => ClientStatusReading {
            command: command.to_string(),
            signed_in: SignedIn::Unknown,
            detail: format!("could not be read: {error}"),
        },
        StatusRun::Result(result) if result.timed_out => ClientStatusReading {
            command: command.to_string(),
            signed_in: SignedIn::Unknown,
            detail: TIMED_OUT.to_string(),
        },
        StatusRun::Result(result) => {
            let verdict = decide(&result);
            ClientStatusReading { command: command.to_string(), signed_in: verdict.signed_in, detail: verdict.detail }
        }
    }
}

/// The first line a command printed that says something, without its indentation.
fn first_line(text: &str) -> Option<&str> {
    text.split('\n').map(str::trim).find(|line| !line.is_empty())
}

/// What a login status prints when it is not logged in:
/// `not logged in|logged out|no credentials|log ?in required|unauthenticated`,
/// case-insensitively.
fn says_not_logged_in(line: &str) -> bool {
    let lower = line.to_lowercase();
    ["not logged in", "logged out", "no credentials", "login required", "log in required", "unauthenticated"]
        .iter()
        .any(|phrase| lower.contains(phrase))
}

/// A login status: `codex login status` prints `Logged in using ChatGPT` and exits
/// 0, and says it is not logged in on a non-zero exit. Only that answer is a
/// client signed out; any other non-zero exit, or one that says nothing, is a
/// command the gateway could not read, which is `unknown`: a client that answers
/// `unrecognized subcommand` is not signed out.
fn login_reading(result: &ProcessResult) -> StatusVerdict {
    let said = first_line(&result.stdout).or_else(|| first_line(&result.stderr));
    if result.exit_code == 0 {
        return StatusVerdict {
            signed_in: SignedIn::Yes,
            detail: said.map_or_else(|| format!("exited {}", result.exit_code), str::to_string),
        };
    }
    if let Some(said) = said {
        if says_not_logged_in(said) {
            return StatusVerdict { signed_in: SignedIn::No, detail: said.to_string() };
        }
    }
    StatusVerdict {
        signed_in: SignedIn::Unknown,
        detail: format!(
            "could not be read: exited {}{}",
            result.exit_code,
            said.map(|said| format!(": {said}")).unwrap_or_default()
        ),
    }
}

/// A credential listing: `hermes auth list` and `opencode auth list` name the
/// credentials the client holds, so a listing that exits 0 and names none is a
/// client with none. Any other non-zero exit is a command the gateway could not
/// read, which is `unknown`: a client that answers `unknown command` is not signed
/// out.
fn listing_reading(result: &ProcessResult) -> StatusVerdict {
    if result.exit_code != 0 {
        let complaint = first_line(&result.stderr);
        return StatusVerdict {
            signed_in: SignedIn::Unknown,
            detail: format!(
                "could not be read: exited {}{}",
                result.exit_code,
                complaint.map(|line| format!(": {line}")).unwrap_or_default()
            ),
        };
    }
    let listed = !result.stdout.trim().is_empty();
    StatusVerdict {
        signed_in: if listed { SignedIn::Yes } else { SignedIn::No },
        detail: if listed { "it lists credentials" } else { "it lists no credentials" }.to_string(),
    }
}
