//! The gate shim (#445, spec #392): a deterministic gate that Gold Band,
//! which has no command node, can still run.
//!
//! A gate node is a cheap worker whose agent id runs exactly one pinned
//! wrapper, `gold-band-gate run`. The wrapper runs the gate command (build,
//! tests, lint, coverage, crate age), captures its output and writes a gate
//! report (`gold-band-report`) that the gate agent cannot write: the report
//! directory is outside what the launcher lets that agent id write (ticket
//! #432, the launcher's OS write limit; this crate only makes the wrapper and
//! the copier correct). The next node is a read-only copier that runs
//! `gold-band-gate verify` and fails the run when the report is missing,
//! stale, from another run or attempt, or does not match, so a faked pass
//! ends in failure. Green means the command ran and exited zero.
//!
//! ```text
//! gold-band-gate run --run-id ID --attempt N --timeout-secs S
//!     --report-dir DIR -- COMMAND [ARGS...]
//! gold-band-gate verify --run-id ID --attempt N --command "COMMAND ARGS"
//!     --report-dir DIR [--max-age-secs N] [--now RFC3339]
//! ```
//!
//! `run` clears `DIR/gate-report.json` and `DIR/gate-output.txt` before the
//! attempt, runs the command with stdout and stderr captured in order into
//! `DIR/gate-output.txt` (at most 8 MiB are kept) and writes the report. The
//! command line recorded is the command and its arguments joined by single
//! spaces. The wrapper exits with the report's `exit_code`:
//!
//! * the command's own exit code;
//! * 124 when it ran longer than `--timeout-secs` and was killed (the report
//!   carries `failed_check` `timeout`);
//! * 127 when it could not start (not found, not executable; `failed_check`
//!   `cannot-start`);
//! * 128 plus the signal number when a signal ended it.
//!
//! Exit 2 from `run` is bad usage or a report directory it cannot clear or
//! write; a command that itself exits 2 earns 2 as well, so the report is
//! the authority and the copier is what a workflow trusts.
//!
//! `verify` only reads. It checks, naming each failure on its own stderr line
//! `gold-band-gate: <check>: <why>`:
//! `missing`, `stale` (older than `--max-age-secs`, default 3600, or more
//! than a minute in the future), `other-run`, `other-attempt`,
//! `command-mismatch`, `hash-mismatch` (SHA-256 recomputed from
//! `gate-output.txt`; a missing output file is a mismatch) and
//! `failed-command` (a non-zero `exit_code`, 124 named as a timeout, or any
//! `failed_check`). Exit 0 prints the report on stdout verbatim, for a node
//! to copy into its success condition; exit 1 prints nothing there; exit 2 is
//! bad usage or a report that cannot be read or is malformed (the report is
//! untrusted: it is size-bounded and every field is type-checked).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use gold_band_report::{parse_rfc3339, rfc3339, sha256_hex, Clock, Report, SystemClock};

/// The crate's name, which prefixes every message line.
pub const NAME: &str = "gold-band-gate";
/// The report file inside the report directory.
pub const REPORT_FILE: &str = "gate-report.json";
/// The captured output file inside the report directory.
pub const OUTPUT_FILE: &str = "gate-output.txt";
/// The exit code of a command that was killed for running too long.
pub const EXIT_TIMEOUT: i32 = 124;
/// The exit code of a command that could not start.
pub const EXIT_CANNOT_START: i32 = 127;
/// The most captured output kept and hashed.
pub const MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
/// The largest report file the copier reads.
pub const MAX_REPORT_BYTES: u64 = 64 * 1024;
/// The age, in seconds, past which the copier calls a report stale.
pub const DEFAULT_MAX_AGE_SECS: u64 = 3600;
/// How far a report timestamp may lie in the future (clock skew), in seconds.
pub const FUTURE_TOLERANCE_SECS: u64 = 60;
/// The longest `--timeout-secs` accepted: one week.
pub const MAX_TIMEOUT_SECS: u64 = 604_800;

const FAILED_TIMEOUT: &str = "timeout";
const FAILED_CANNOT_START: &str = "cannot-start";

/// How one gate command ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It ran to the end with this exit code (128 plus the signal number
    /// when a signal ended it).
    Exited(i32),
    /// It was still running at the deadline and was killed.
    TimedOut,
    /// It could not be started; the reason.
    CannotStart(String),
}

/// The outside world the wrapper reaches for. Tests fake it; [`RealHost`]
/// is the real one.
pub trait Host: Clock {
    /// Runs `argv` with stdin closed and stdout and stderr appended, in
    /// order, to the existing file `output`; kills it after `timeout`.
    fn execute(&self, argv: &[String], timeout: Duration, output: &Path) -> Outcome;
    /// Prints text on stdout, or on stderr when `stderr` is set.
    fn print(&self, stderr: bool, text: &str);
}

/// The real host: child processes, the standard streams and the system clock.
pub struct RealHost;

impl Clock for RealHost {
    fn unix_seconds(&self) -> u64 {
        SystemClock.unix_seconds()
    }
}

impl Host for RealHost {
    fn execute(&self, argv: &[String], timeout: Duration, output: &Path) -> Outcome {
        match spawn_and_wait(argv, timeout, output) {
            Ok(outcome) => outcome,
            Err(e) => Outcome::CannotStart(e.to_string()),
        }
    }

    fn print(&self, stderr: bool, text: &str) {
        if stderr {
            eprint!("{text}");
        } else {
            print!("{text}");
        }
    }
}

/// Starts the command in its own process group and waits for it until the
/// deadline; on expiry the whole group is killed and reaped.
fn spawn_and_wait(argv: &[String], timeout: Duration, output: &Path) -> io::Result<Outcome> {
    let out = OpenOptions::new().append(true).open(output)?;
    let err = out.try_clone()?;
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Outcome::Exited(exit_code(status)));
        }
        if Instant::now() >= deadline {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
            child.wait()?;
            return Ok(Outcome::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The exit code of a finished child: its own, or 128 plus the signal (1
/// when neither is known).
fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| status.signal().map_or(1, |signal| 128 + signal))
}

/// A problem line: the check that raised it and why.
struct Problem {
    check: &'static str,
    text: String,
}

fn problem(check: &'static str, text: String) -> Problem {
    Problem { check, text }
}

/// The usage text a bad command line prints on stderr.
fn usage() -> String {
    format!(
        "usage: {NAME} run --run-id ID --attempt N --timeout-secs S --report-dir DIR -- COMMAND [ARGS...]\n       {NAME} verify --run-id ID --attempt N --command \"COMMAND ARGS\" --report-dir DIR [--max-age-secs N] [--now RFC3339]"
    )
}

const RUN_FLAGS: [&str; 4] = ["run-id", "attempt", "timeout-secs", "report-dir"];
const VERIFY_FLAGS: [&str; 6] = [
    "run-id",
    "attempt",
    "command",
    "report-dir",
    "max-age-secs",
    "now",
];

/// Splits arguments into value flags and, when `command_follows`, the
/// command after `--`. Refuses an unknown or repeated flag and a flag
/// without a value.
fn split_flags(
    args: &[String],
    allowed: &[&str],
    command_follows: bool,
) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
    let mut found = BTreeMap::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if command_follows && arg == "--" {
            return Ok((found, rest.cloned().collect()));
        }
        let name = arg
            .strip_prefix("--")
            .filter(|name| allowed.contains(name))
            .ok_or_else(|| format!("unexpected argument {arg}"))?;
        let value = rest
            .next()
            .ok_or_else(|| format!("--{name} needs a value"))?;
        if found.insert(name.to_string(), value.clone()).is_some() {
            return Err(format!("--{name} given twice"));
        }
    }
    Ok((found, Vec::new()))
}

/// A required flag, non-empty.
fn required(found: &BTreeMap<String, String>, name: &str) -> Result<String, String> {
    match found.get(name) {
        Some(v) if !v.is_empty() => Ok(v.clone()),
        Some(_) => Err(format!("--{name} must not be empty")),
        None => Err(format!("--{name} is required")),
    }
}

/// A non-negative integer flag.
fn number(text: &str, name: &str) -> Result<u64, String> {
    text.parse()
        .map_err(|_| format!("--{name} must be a non-negative integer"))
}

/// The common identity flags: run id, attempt and report directory.
struct Identity {
    run_id: String,
    attempt: u32,
    report_dir: String,
}

fn identity(found: &BTreeMap<String, String>) -> Result<Identity, String> {
    let attempt = u32::try_from(number(&required(found, "attempt")?, "attempt")?)
        .map_err(|_| "--attempt is too large".to_string())?;
    Ok(Identity {
        run_id: required(found, "run-id")?,
        attempt,
        report_dir: required(found, "report-dir")?,
    })
}

/// What `run` was asked to do.
struct Wrap {
    id: Identity,
    timeout: Duration,
    command: Vec<String>,
}

/// What `verify` was asked to check.
struct Check {
    id: Identity,
    command: String,
    max_age: u64,
    now: Option<u64>,
}

enum Call {
    Wrap(Wrap),
    Check(Check),
}

/// The typed call out of the command line.
fn parse(args: &[String]) -> Result<Call, String> {
    match args.first().map(String::as_str) {
        Some("run") => {
            let (found, command) = split_flags(&args[1..], &RUN_FLAGS, true)?;
            if command.is_empty() {
                return Err("a command is required after --".to_string());
            }
            let secs = number(&required(&found, "timeout-secs")?, "timeout-secs")?;
            if !(1..=MAX_TIMEOUT_SECS).contains(&secs) {
                return Err(format!(
                    "--timeout-secs must be between 1 and {MAX_TIMEOUT_SECS}"
                ));
            }
            Ok(Call::Wrap(Wrap {
                id: identity(&found)?,
                timeout: Duration::from_secs(secs),
                command,
            }))
        }
        Some("verify") => {
            let (found, _) = split_flags(&args[1..], &VERIFY_FLAGS, false)?;
            let max_age = match found.get("max-age-secs") {
                None => DEFAULT_MAX_AGE_SECS,
                Some(text) => number(text, "max-age-secs")?,
            };
            let now = match found.get("now") {
                None => None,
                Some(text) => Some(parse_rfc3339(text).ok_or_else(|| {
                    "--now must be RFC 3339 UTC like 2026-11-11T09:30:00Z".to_string()
                })?),
            };
            Ok(Call::Check(Check {
                id: identity(&found)?,
                command: required(&found, "command")?,
                max_age,
                now,
            }))
        }
        _ => Err("the subcommands are run and verify".to_string()),
    }
}

/// Runs one command line against the real world and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    run_with(args, &RealHost)
}

/// [`run`] against any [`Host`].
pub fn run_with<I>(args: I, host: &dyn Host) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let result = match parse(&args) {
        Err(why) => {
            host.print(true, &format!("{NAME}: {why}\n{}\n", usage()));
            return 2;
        }
        Ok(Call::Wrap(call)) => wrap(&call, host),
        Ok(Call::Check(call)) => verify(&call, host),
    };
    result.unwrap_or_else(|why| {
        host.print(true, &format!("{NAME}: {why}\n"));
        2
    })
}

/// `run`: clears the previous attempt, runs the command, writes the report
/// and returns the report's exit code.
fn wrap(call: &Wrap, host: &dyn Host) -> Result<i32, String> {
    let dir = Path::new(&call.id.report_dir);
    let (report_path, output_path) = (dir.join(REPORT_FILE), dir.join(OUTPUT_FILE));
    let prepare = || -> io::Result<()> {
        fs::create_dir_all(dir)?;
        for stale in [&report_path, &output_path] {
            match fs::remove_file(stale) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        fs::write(&output_path, b"")
    };
    prepare().map_err(|e| format!("{}: cannot prepare the attempt: {e}", dir.display()))?;
    let command = call.command.join(" ");
    let (exit_code, failed_check) = match host.execute(&call.command, call.timeout, &output_path) {
        Outcome::Exited(code) => (code, None),
        Outcome::TimedOut => {
            let note = format!(
                "{NAME}: timed out after {} seconds and was killed\n",
                call.timeout.as_secs()
            );
            append(&output_path, &note)?;
            (EXIT_TIMEOUT, Some(FAILED_TIMEOUT))
        }
        Outcome::CannotStart(why) => {
            append(&output_path, &format!("{NAME}: cannot start: {why}\n"))?;
            (EXIT_CANNOT_START, Some(FAILED_CANNOT_START))
        }
    };
    let output = capture(&output_path)
        .map_err(|e| format!("{}: cannot read: {e}", output_path.display()))?;
    let report = Report {
        run_id: call.id.run_id.clone(),
        attempt: call.id.attempt,
        command,
        exit_code,
        output_hash: sha256_hex(&output),
        timestamp: rfc3339(host.unix_seconds()),
        failed_check: failed_check.map(str::to_string),
    };
    gold_band_report::write(dir, REPORT_FILE, &report)
        .map_err(|e| format!("{}: cannot write: {e}", report_path.display()))?;
    host.print(
        false,
        &format!(
            "{NAME}: {} exit {exit_code}{}\n",
            report.command,
            failed_check.map_or(String::new(), |c| format!(" ({c})"))
        ),
    );
    Ok(exit_code)
}

fn append(path: &Path, text: &str) -> Result<(), String> {
    let write = || -> io::Result<()> {
        use std::io::Write;
        OpenOptions::new()
            .append(true)
            .open(path)?
            .write_all(text.as_bytes())
    };
    write().map_err(|e| format!("{}: cannot write: {e}", path.display()))
}

/// The captured output, cut to [`MAX_OUTPUT_BYTES`] on disk and in memory.
fn capture(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_OUTPUT_BYTES {
        bytes.truncate(MAX_OUTPUT_BYTES as usize);
        OpenOptions::new()
            .write(true)
            .open(path)?
            .set_len(MAX_OUTPUT_BYTES)?;
    }
    Ok(bytes)
}

/// Reads at most `limit` bytes of `path`; `Ok(None)` when it is larger.
fn read_bounded(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

/// `verify`: reads the report and returns 0 (report printed), 1 (problems
/// printed) or an error for exit 2. Nothing is written to disk.
fn verify(call: &Check, host: &dyn Host) -> Result<i32, String> {
    let dir = Path::new(&call.id.report_dir);
    let report_path = dir.join(REPORT_FILE);
    let shown = report_path.display();
    let bytes = match read_bounded(&report_path, MAX_REPORT_BYTES) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Err(format!("{shown}: larger than {MAX_REPORT_BYTES} bytes")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let missing = [problem("missing", format!("no gate report at {shown}"))];
            return Ok(refuse(host, &missing));
        }
        Err(e) => return Err(format!("{shown}: cannot read: {e}")),
    };
    let text = String::from_utf8(bytes).map_err(|_| format!("{shown}: not UTF-8"))?;
    let report = Report::from_json(&text).map_err(|e| format!("{shown}: {e}"))?;
    let now = call.now.unwrap_or_else(|| host.unix_seconds());
    let problems = report_problems(call, &report, now, dir);
    if problems.is_empty() {
        host.print(false, &text);
        return Ok(0);
    }
    Ok(refuse(host, &problems))
}

fn refuse(host: &dyn Host, problems: &[Problem]) -> i32 {
    let lines: String = problems
        .iter()
        .map(|p| format!("{NAME}: {}: {}\n", p.check, p.text))
        .collect();
    host.print(true, &lines);
    1
}

/// Every way the report fails to prove this run's gate ran and passed.
fn report_problems(call: &Check, report: &Report, now: u64, dir: &Path) -> Vec<Problem> {
    let mut problems = Vec::new();
    if report.run_id != call.id.run_id {
        problems.push(problem(
            "other-run",
            format!(
                "the report is for run {:?}, not {:?}",
                report.run_id, call.id.run_id
            ),
        ));
    }
    if report.attempt != call.id.attempt {
        problems.push(problem(
            "other-attempt",
            format!(
                "the report is for attempt {}, not {}",
                report.attempt, call.id.attempt
            ),
        ));
    }
    if report.command != call.command {
        problems.push(problem(
            "command-mismatch",
            format!(
                "the report ran {:?}, not {:?}",
                report.command, call.command
            ),
        ));
    }
    // from_json already proved the timestamp parses.
    let stamped = parse_rfc3339(&report.timestamp).unwrap_or(0);
    if stamped > now.saturating_add(FUTURE_TOLERANCE_SECS) {
        problems.push(problem(
            "stale",
            format!("the report is stamped {}, in the future", report.timestamp),
        ));
    } else if now > stamped.saturating_add(call.max_age) {
        problems.push(problem(
            "stale",
            format!(
                "the report is stamped {}, more than {} seconds old",
                report.timestamp, call.max_age
            ),
        ));
    }
    problems.extend(hash_problem(report, dir));
    if report.exit_code != 0 || report.failed_check.is_some() {
        let how = match (report.exit_code, &report.failed_check) {
            (EXIT_TIMEOUT, _) => " (the command timed out)".to_string(),
            (_, Some(check)) => format!(" ({check})"),
            _ => String::new(),
        };
        problems.push(problem(
            "failed-command",
            format!("the command exited {}{how}", report.exit_code),
        ));
    }
    problems
}

/// The `hash-mismatch` problem, if the captured output is missing, too large
/// or hashes to something else.
fn hash_problem(report: &Report, dir: &Path) -> Option<Problem> {
    let path = dir.join(OUTPUT_FILE);
    let why = match read_bounded(&path, MAX_OUTPUT_BYTES) {
        Err(e) => format!("{} cannot be read: {e}", path.display()),
        Ok(None) => format!("{} is larger than {MAX_OUTPUT_BYTES} bytes", path.display()),
        Ok(Some(bytes)) if sha256_hex(&bytes) != report.output_hash => format!(
            "{} no longer hashes to the report's output_hash",
            path.display()
        ),
        Ok(Some(_)) => return None,
    };
    Some(problem("hash-mismatch", why))
}

#[cfg(test)]
mod tests;
