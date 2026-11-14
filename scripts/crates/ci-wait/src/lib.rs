//! ci-wait: wait until every CI check of one commit is terminal, and print one
//! JSON object with the verdict.
//!
//! ```text
//! ci-wait --repo-dir DIR --sha SHA [--repo OWNER/NAME] [--timeout-secs N] [--poll-secs N]
//! ```
//!
//! One look is one `GET repos/OWNER/NAME/commits/SHA/check-runs`, paged until
//! `total_count` runs are collected. A run is terminal when its status is
//! `completed`; `success`, `neutral` and `skipped` pass and every other
//! conclusion fails. A failed job the repository's `docs/agents/setup.yaml`
//! lists under `flaky_jobs` is rerun once per invocation; any other failure is
//! red at once. Green needs every run terminal and passing on two consecutive
//! looks, so a check that is still being registered cannot pass for it.
//!
//! Exit 0 is green, 1 is red, and 2 is bad usage or a refused repository, which
//! prints its message on stderr and no JSON. GitHub and the clock sit behind
//! the [`GitHub`] and [`Clock`] traits: tests fake them, and [`GhCli`] and
//! [`SystemClock`] are the real ones.

pub mod setup;

#[cfg(test)]
mod tests;

use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The page size asked of the check-run list.
const PAGE: usize = 100;

/// The usage text a bad command line prints on stderr.
const USAGE: &str = "usage: ci-wait --repo-dir DIR --sha SHA [--repo OWNER/NAME] [--timeout-secs N] [--poll-secs N]\n";

/// The value flags the command line takes.
const FLAGS: [&str; 5] = ["repo-dir", "sha", "repo", "timeout-secs", "poll-secs"];

/// The wait's default deadline, in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 3600;

/// The default pause between looks, in seconds.
const DEFAULT_POLL_SECS: u64 = 30;

/// How often a running `gh` is asked whether it has finished.
const POLL: Duration = Duration::from_millis(10);

/// The conclusions that pass; every other terminal conclusion fails.
const PASSING: [&str; 3] = ["success", "neutral", "skipped"];

/// The two things this tool may do to GitHub: read a path, and rerun a job.
pub trait GitHub {
    /// The raw JSON body of `GET <path>`, or why it could not be read.
    fn get(&mut self, path: &str) -> Result<String, String>;

    /// The raw JSON body of `POST <path>`, or why it could not be posted.
    fn post(&mut self, path: &str) -> Result<String, String>;
}

/// The wait's clock: how long it has been waiting, and how to pause.
pub trait Clock {
    /// The time since the start of the wait.
    fn elapsed(&self) -> Duration;

    /// Pauses for `d`.
    fn sleep(&mut self, d: Duration);
}

/// The real seam: one `gh api -X <METHOD> <path>` per call, with a deadline.
#[derive(Debug)]
pub struct GhCli {
    program: OsString,
    timeout: Duration,
}

impl GhCli {
    /// The real `gh`, with a minute per call.
    pub fn new() -> Self {
        Self::with(OsString::from("gh"), Duration::from_secs(60))
    }

    /// The same client over another program and deadline (the tests point it
    /// at a script).
    pub fn with(program: OsString, timeout: Duration) -> Self {
        GhCli { program, timeout }
    }

    /// One `gh api` call: the body on success, the failure message otherwise.
    fn call(&mut self, method: &str, path: &str) -> Result<String, String> {
        let mut child = Command::new(&self.program)
            .arg("api")
            .arg("-X")
            .arg(method)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", self.program.to_string_lossy()))?;
        // Both pipes are read on their own thread, so a large answer cannot
        // fill a pipe buffer and deadlock the wait below.
        let out_reader = child.stdout.take().map(reader);
        let err_reader = child.stderr.take().map(reader);
        let status = wait(&mut child, self.timeout)?;
        // Both pipes were asked for, so both readers are there.
        let out = out_reader.map(|handle| handle.join().unwrap_or_default()).unwrap_or_default();
        let err = err_reader.map(|handle| handle.join().unwrap_or_default()).unwrap_or_default();
        if status.success() {
            return Ok(out);
        }
        let message = err.trim();
        Err(format!("gh api -X {method} {path}: {message}"))
    }
}

impl Default for GhCli {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHub for GhCli {
    fn get(&mut self, path: &str) -> Result<String, String> {
        self.call("GET", path)
    }

    fn post(&mut self, path: &str) -> Result<String, String> {
        self.call("POST", path)
    }
}

/// Drain one pipe to a string on its own thread. A read error leaves what was
/// read so far, which then fails the shape check rather than the read.
fn reader<R: Read + Send + 'static>(mut pipe: R) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Wait for the child, killing it once the deadline has passed.
fn wait(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("gh timed out".to_string());
        }
        std::thread::sleep(POLL);
    }
}

/// The real clock: an `Instant` at the start of the wait, and the thread's own
/// sleep.
#[derive(Debug)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// A clock whose wait starts now.
    pub fn new() -> Self {
        SystemClock {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// One check run, as the list answers carry it.
#[derive(Clone, Debug)]
struct Run {
    id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
}

/// One job in the printed answer.
struct Job {
    name: String,
    status: String,
    conclusion: Option<String>,
    reran: bool,
}

/// One rerun in the printed answer, with the reason the setup file gives.
#[derive(Clone)]
struct Rerun {
    job: String,
    reason: String,
}

/// The answer: the verdict, why, and what the last look saw.
struct Verdict {
    green: bool,
    reason: Option<String>,
    jobs: Vec<Job>,
    reruns: Vec<Rerun>,
}

/// Look until the checks are terminal, rerunning a listed flaky job once.
fn watch(
    gh: &mut dyn GitHub,
    clock: &mut dyn Clock,
    repo: &str,
    sha: &str,
    flaky: &[setup::FlakyJob],
    timeout: Duration,
    poll: Duration,
) -> Verdict {
    let mut rerun_names: Vec<String> = Vec::new();
    let mut rerun_ids: Vec<(String, u64)> = Vec::new();
    let mut reruns: Vec<Rerun> = Vec::new();
    let mut last: Vec<Run> = Vec::new();
    let mut previous: Option<Vec<(u64, String, Option<String>)>> = None;
    let mut seen = false;
    loop {
        let runs = match look(gh, repo, sha) {
            Ok(runs) => runs,
            Err(message) => {
                let reason = format!("unreadable: {message}");
                return verdict_of(false, Some(reason), &last, &rerun_names, &reruns);
            }
        };
        if !runs.is_empty() {
            seen = true;
        }
        last = runs.clone();
        // A failure that may not be rerun is red at once; a listed flaky
        // failure is collected and rerun below, so a look never both reruns
        // and decides.
        let mut to_rerun: Vec<(&Run, &str)> = Vec::new();
        for run in &runs {
            if !is_failed(run) || is_pending(run, &rerun_ids) {
                continue;
            }
            match flaky.iter().find(|job| job.name == run.name) {
                Some(job) if !rerun_names.contains(&run.name) => {
                    to_rerun.push((run, job.reason.as_str()))
                }
                _ => {
                    let reason = format!("failed: {}", run.name);
                    return verdict_of(false, Some(reason), &last, &rerun_names, &reruns);
                }
            }
        }
        let mut reran = false;
        for (run, reason) in to_rerun {
            let path = format!("repos/{repo}/actions/jobs/{}/rerun", run.id);
            match gh.post(&path) {
                Ok(_) => {
                    rerun_names.push(run.name.clone());
                    rerun_ids.push((run.name.clone(), run.id));
                    reruns.push(Rerun {
                        job: run.name.clone(),
                        reason: reason.to_string(),
                    });
                    reran = true;
                }
                Err(message) => {
                    let reason = format!("rerun failed: {}: {message}", run.name);
                    return verdict_of(false, Some(reason), &last, &rerun_names, &reruns);
                }
            }
        }
        if reran {
            previous = None;
        } else if all_passing(&runs) {
            let signature = signature(&runs);
            if previous.as_ref() == Some(&signature) {
                return verdict_of(true, None, &last, &rerun_names, &reruns);
            }
            previous = Some(signature);
        } else {
            previous = None;
        }
        if clock.elapsed() >= timeout {
            let reason = if seen {
                "timed out"
            } else {
                "no checks registered"
            };
            return verdict_of(
                false,
                Some(reason.to_string()),
                &last,
                &rerun_names,
                &reruns,
            );
        }
        clock.sleep(poll);
    }
}

/// One look: every check run of the commit, page by page.
fn look(gh: &mut dyn GitHub, repo: &str, sha: &str) -> Result<Vec<Run>, String> {
    let mut runs: Vec<Run> = Vec::new();
    let mut page = 1;
    loop {
        let path = format!("repos/{repo}/commits/{sha}/check-runs?per_page={PAGE}&page={page}");
        let body = gh.get(&path)?;
        let value: Value = serde_json::from_str(&body).map_err(|e| format!("{path}: {e}"))?;
        let total = value
            .get("total_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let items = value
            .get("check_runs")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{path}: no check_runs list"))?;
        if items.is_empty() {
            break;
        }
        for item in items {
            runs.push(parse_run(item, &path)?);
        }
        if runs.len() as u64 >= total {
            break;
        }
        page += 1;
    }
    Ok(runs)
}

/// One run out of the list, or why the answer is not the shape GitHub promises.
fn parse_run(item: &Value, path: &str) -> Result<Run, String> {
    let id = item
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{path}: a check run without an id"))?;
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{path}: a check run without a name"))?;
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{path}: a check run without a status"))?;
    let conclusion = match item.get("conclusion") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(_) => return Err(format!("{path}: a conclusion that is not a string")),
    };
    Ok(Run {
        id,
        name: name.to_string(),
        status: status.to_string(),
        conclusion,
    })
}

/// Whether a run is terminal and did not pass.
fn is_failed(run: &Run) -> bool {
    run.status == "completed" && !passes(run.conclusion.as_deref())
}

/// Whether a run is the one already rerun, still listed as it was: pending,
/// not failed, until a run with a different id appears.
fn is_pending(run: &Run, rerun_ids: &[(String, u64)]) -> bool {
    rerun_ids
        .iter()
        .any(|(name, id)| name == &run.name && *id == run.id)
}

/// Whether every run is terminal and passing; an empty list is not.
fn all_passing(runs: &[Run]) -> bool {
    !runs.is_empty()
        && runs
            .iter()
            .all(|run| run.status == "completed" && passes(run.conclusion.as_deref()))
}

/// What a look saw, for the two-look rule.
fn signature(runs: &[Run]) -> Vec<(u64, String, Option<String>)> {
    runs.iter()
        .map(|run| (run.id, run.name.clone(), run.conclusion.clone()))
        .collect()
}

/// Whether a conclusion passes.
fn passes(conclusion: Option<&str>) -> bool {
    matches!(conclusion, Some(text) if PASSING.contains(&text))
}

/// The answer for one outcome, with the last look's jobs.
fn verdict_of(
    green: bool,
    reason: Option<String>,
    runs: &[Run],
    rerun_names: &[String],
    reruns: &[Rerun],
) -> Verdict {
    let jobs = runs
        .iter()
        .map(|run| Job {
            name: run.name.clone(),
            status: run.status.clone(),
            conclusion: run.conclusion.clone(),
            reran: rerun_names.contains(&run.name),
        })
        .collect();
    Verdict {
        green,
        reason,
        jobs,
        reruns: reruns.to_vec(),
    }
}

/// The one JSON line, keys in the order the contract fixes.
fn render(verdict: &Verdict, repo: &str, sha: &str) -> String {
    let mut fields = vec![format!(
        "\"verdict\":{}",
        text(if verdict.green { "green" } else { "red" })
    )];
    if let Some(reason) = &verdict.reason {
        fields.push(format!("\"reason\":{}", text(reason)));
    }
    fields.push(format!("\"repo\":{}", text(repo)));
    fields.push(format!("\"sha\":{}", text(sha)));
    let jobs: Vec<String> = verdict
        .jobs
        .iter()
        .map(|job| {
            let conclusion = match &job.conclusion {
                Some(conclusion) => text(conclusion),
                None => "null".to_string(),
            };
            format!(
                "{{\"name\":{},\"status\":{},\"conclusion\":{conclusion},\"reran\":{}}}",
                text(&job.name),
                text(&job.status),
                job.reran
            )
        })
        .collect();
    fields.push(format!("\"jobs\":[{}]", jobs.join(",")));
    let reruns: Vec<String> = verdict
        .reruns
        .iter()
        .map(|rerun| {
            format!(
                "{{\"job\":{},\"reason\":{}}}",
                text(&rerun.job),
                text(&rerun.reason)
            )
        })
        .collect();
    fields.push(format!("\"reruns\":[{}]", reruns.join(",")));
    format!("{{{}}}", fields.join(","))
}

/// One JSON string value, escaped by serde_json.
fn text(value: &str) -> String {
    Value::String(value.to_string()).to_string()
}

/// The typed command line.
struct Options {
    repo_dir: PathBuf,
    sha: String,
    repo: Option<String>,
    timeout: Duration,
    poll: Duration,
}

/// The value flags, refusing an unknown or repeated one and a flag without a
/// value.
fn flags(args: &[OsString]) -> Result<BTreeMap<String, OsString>, String> {
    let mut found = BTreeMap::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let name = arg
            .to_str()
            .and_then(|text| text.strip_prefix("--"))
            .filter(|name| FLAGS.contains(name))
            .ok_or_else(|| format!("unexpected argument {}", arg.to_string_lossy()))?;
        let value = rest
            .next()
            .ok_or_else(|| format!("--{name} needs a value"))?;
        if found.insert(name.to_string(), value.clone()).is_some() {
            return Err(format!("--{name} given twice"));
        }
    }
    Ok(found)
}

/// The typed options out of the flags.
fn options(found: &BTreeMap<String, OsString>) -> Result<Options, String> {
    let text_of = |name: &str| -> Result<String, String> {
        let value = found
            .get(name)
            .ok_or_else(|| format!("--{name} is required"))?;
        value
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| format!("--{name} must be UTF-8"))
    };
    let repo_dir = PathBuf::from(
        found
            .get("repo-dir")
            .ok_or_else(|| "--repo-dir is required".to_string())?,
    );
    let sha = text_of("sha")?;
    if !is_sha(&sha) {
        return Err(format!("--sha must be 7 to 64 hex characters: {sha}"));
    }
    let repo = match found.get("repo") {
        None => None,
        Some(_) => {
            let repo = text_of("repo")?;
            if !is_owner_name(&repo) {
                return Err(format!("--repo must be OWNER/NAME: {repo}"));
            }
            Some(repo)
        }
    };
    let timeout = match found.get("timeout-secs") {
        None => DEFAULT_TIMEOUT_SECS,
        Some(_) => number("timeout-secs", &text_of("timeout-secs")?)?,
    };
    let poll = match found.get("poll-secs") {
        None => DEFAULT_POLL_SECS,
        Some(_) => number("poll-secs", &text_of("poll-secs")?)?,
    };
    Ok(Options {
        repo_dir,
        sha,
        repo,
        timeout: Duration::from_secs(timeout),
        poll: Duration::from_secs(poll),
    })
}

/// A whole number flag value.
fn number(name: &str, text: &str) -> Result<u64, String> {
    text.parse::<u64>()
        .map_err(|_| format!("--{name} must be a whole number: {text}"))
}

/// Whether `sha` is 7 to 64 hex characters.
fn is_sha(sha: &str) -> bool {
    (7..=64).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether `repo` is `OWNER/NAME`: exactly one slash, each part letters,
/// digits, `_`, `.` or `-`, and neither part only dots (so no `./` or `../`).
pub(crate) fn is_owner_name(repo: &str) -> bool {
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) => is_name_part(owner) && is_name_part(name),
        _ => false,
    }
}

fn is_name_part(part: &str) -> bool {
    !part.is_empty()
        && !part.chars().all(|c| c == '.')
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// The command over an injected GitHub and clock: returns the exit status.
pub fn run_with<I: IntoIterator<Item = OsString>>(
    args: I,
    gh: &mut dyn GitHub,
    clock: &mut dyn Clock,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let args: Vec<OsString> = args.into_iter().collect();
    let opts = match flags(&args).and_then(|found| options(&found)) {
        Ok(opts) => opts,
        Err(message) => {
            let _ = writeln!(err, "{message}");
            let _ = write!(err, "{USAGE}");
            return 2;
        }
    };
    let file = match setup::load(&opts.repo_dir) {
        Ok(file) => file,
        Err(message) => {
            let _ = writeln!(err, "{message}");
            return 2;
        }
    };
    let repo = opts.repo.unwrap_or_else(|| file.tracker_repo.clone());
    let answer = watch(
        gh,
        clock,
        &repo,
        &opts.sha,
        &file.flaky_jobs,
        opts.timeout,
        opts.poll,
    );
    let line = render(&answer, &repo, &opts.sha);
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
    if answer.green {
        0
    } else {
        1
    }
}

/// The command over the real `gh` and the real clock.
pub fn run<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
    run_with(
        args,
        &mut GhCli::new(),
        &mut SystemClock::new(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}
