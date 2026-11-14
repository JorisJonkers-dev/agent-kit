//! ticket-export: export a published GitHub issue and its sub-issues to the
//! requirement file Workflow B reads, through read-only GitHub access.
//!
//! One read is one `gh api -X GET <path>`. The [`GitHub`] trait has no other
//! method, so an implementor can only read; the tracker repository comes from
//! the repository's own `docs/agents/setup.yaml` ([`setup`]).

pub mod setup;

#[cfg(test)]
mod tests;

use serde_json::Value;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The page size asked of the list reads; a full page may hide more, so it is refused.
const PAGE: usize = 100;

const USAGE: &str = "usage: ticket-export <REPO_DIR> <ISSUE_NUMBER> <OUT_FILE>\n";

/// How often a running `gh` is asked whether it has finished.
const POLL: Duration = Duration::from_millis(10);

/// Why a read did not produce an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// GitHub answered that the thing is not there.
    NotFound,
    /// The read could not be made, or `gh` failed for another reason.
    Unreadable(String),
}

/// The one thing this tool may do to GitHub: read a path and get its raw JSON
/// body back.
pub trait GitHub {
    fn get(&mut self, path: &str) -> Result<String, ReadError>;
}

/// The real seam: one `gh api -X GET <path>` per read, with a deadline.
#[derive(Debug)]
pub struct GhCli {
    program: OsString,
    timeout: Duration,
}

impl GhCli {
    pub fn new() -> Self {
        Self::with(OsString::from("gh"), Duration::from_secs(60))
    }

    /// The same client over another program and deadline (the tests point it
    /// at a script).
    pub fn with(program: OsString, timeout: Duration) -> Self {
        GhCli { program, timeout }
    }
}

impl Default for GhCli {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHub for GhCli {
    fn get(&mut self, path: &str) -> Result<String, ReadError> {
        let mut child = Command::new(&self.program)
            .arg("api")
            .arg("-X")
            .arg("GET")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                let program = self.program.to_string_lossy();
                ReadError::Unreadable(format!("{program}: {e}"))
            })?;
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
        if err.contains("HTTP 404") {
            return Err(ReadError::NotFound);
        }
        let message = err.trim();
        Err(ReadError::Unreadable(format!(
            "gh api -X GET {path}: {message}"
        )))
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
fn wait(child: &mut Child, timeout: Duration) -> Result<ExitStatus, ReadError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let seconds = timeout.as_secs_f64();
            return Err(ReadError::Unreadable(format!("timed out after {seconds}s")));
        }
        std::thread::sleep(POLL);
    }
}

/// A linked issue, as the list answers carry it.
#[derive(Debug)]
struct Link {
    number: u64,
    title: String,
    state: String,
}

/// Read the issue, its sub-issues and its blockers, and render the requirement
/// document. Nothing is returned on a refusal, so nothing is written.
pub fn export(gh: &mut dyn GitHub, repo: &str, number: u64) -> Result<String, String> {
    let issue_path = format!("repos/{repo}/issues/{number}");
    let sub_path = format!("repos/{repo}/issues/{number}/sub_issues?per_page={PAGE}");
    let blocked_path =
        format!("repos/{repo}/issues/{number}/dependencies/blocked_by?per_page={PAGE}");
    let issue_text = fetch(
        gh,
        &issue_path,
        format!("issue #{number} in {repo} not found"),
        number,
        repo,
    )?;
    let sub_text = fetch(
        gh,
        &sub_path,
        format!("the sub-issues of issue #{number} in {repo} not found"),
        number,
        repo,
    )?;
    let blocked_text = fetch(
        gh,
        &blocked_path,
        format!("the blocked-by issues of issue #{number} in {repo} not found"),
        number,
        repo,
    )?;
    let issue = parse(&issue_text, number, repo)?;
    let title = match (
        issue.get("title").and_then(Value::as_str),
        issue.get("state").and_then(Value::as_str),
    ) {
        (Some(title), Some(_state)) => title,
        _ => return Err(unexpected(number, repo)),
    };
    let body = issue
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let sub_issues = links(&sub_text, number, repo)?;
    let blocked_by = links(&blocked_text, number, repo)?;
    for blocker in &blocked_by {
        if blocker.state == "closed" {
            let blocked = blocker.number;
            return Err(format!(
                "issue #{number} in {repo} is blocked by closed issue #{blocked}"
            ));
        }
    }
    Ok(render(
        number,
        title,
        &normalise(body),
        &sub_issues,
        &blocked_by,
    ))
}

/// One read, with the refusal message for each way it can fail.
fn fetch(
    gh: &mut dyn GitHub,
    path: &str,
    not_found: String,
    number: u64,
    repo: &str,
) -> Result<String, String> {
    match gh.get(path) {
        Ok(text) => Ok(text),
        Err(ReadError::NotFound) => Err(not_found),
        Err(ReadError::Unreadable(message)) => {
            let refusal = format!("cannot read issue #{number} in {repo}: {message}");
            Err(refusal)
        }
    }
}

fn unexpected(number: u64, repo: &str) -> String {
    format!("issue #{number} in {repo}: unexpected answer")
}

fn parse(text: &str, number: u64, repo: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(text).map_err(|_| unexpected(number, repo))
}

fn links(text: &str, number: u64, repo: &str) -> Result<Vec<Link>, String> {
    let value = parse(text, number, repo)?;
    let items = value.as_array().ok_or_else(|| unexpected(number, repo))?;
    if items.len() >= PAGE {
        return Err(format!(
            "issue #{number} in {repo}: {PAGE} or more linked issues, more than one page"
        ));
    }
    let mut found = Vec::new();
    for item in items {
        let fields = (
            item.get("number").and_then(Value::as_u64),
            item.get("title").and_then(Value::as_str),
            item.get("state").and_then(Value::as_str),
        );
        match fields {
            (Some(linked), Some(title), Some(state)) => found.push(Link {
                number: linked,
                title: title.to_string(),
                state: state.to_string(),
            }),
            _ => return Err(unexpected(number, repo)),
        }
    }
    Ok(found)
}

/// The body as the document carries it: `\n` line endings, no trailing space.
fn normalise(body: &str) -> String {
    body.replace("\r\n", "\n").trim_end().to_string()
}

/// The lines under the body's `## Acceptance criteria` heading, up to the next
/// `## ` heading.
fn acceptance_criteria(body: &str) -> Option<String> {
    let mut found = false;
    let mut lines: Vec<&str> = Vec::new();
    for line in body.lines() {
        if found {
            if line.starts_with("## ") {
                break;
            }
            lines.push(line);
        } else if line.trim_end() == "## Acceptance criteria" {
            found = true;
        }
    }
    if !found {
        return None;
    }
    while matches!(lines.first(), Some(line) if line.trim().is_empty()) {
        lines.remove(0);
    }
    while matches!(lines.last(), Some(line) if line.trim().is_empty()) {
        lines.pop();
    }
    Some(lines.join("\n")).filter(|text| !text.is_empty())
}

fn list(links: &[Link]) -> String {
    if links.is_empty() {
        return "None".to_string();
    }
    links
        .iter()
        .map(|link| {
            let Link {
                number,
                title,
                state,
            } = link;
            format!("- #{number} {title} ({state})")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render(
    number: u64,
    title: &str,
    body: &str,
    sub_issues: &[Link],
    blocked_by: &[Link],
) -> String {
    let mut blocks = vec![format!("# {title} (#{number})")];
    if !body.is_empty() {
        blocks.push(format!("## Body\n\n{body}"));
    }
    let criteria = acceptance_criteria(body).unwrap_or_else(|| "None".to_string());
    let sub = list(sub_issues);
    let blocked = list(blocked_by);
    blocks.push(format!("## Acceptance criteria\n\n{criteria}"));
    blocks.push(format!("## Sub-issues\n\n{sub}"));
    blocks.push(format!("## Blocked by\n\n{blocked}"));
    blocks.join("\n\n") + "\n"
}

/// `<REPO_DIR> <ISSUE_NUMBER> <OUT_FILE>`: exactly three arguments, a positive
/// issue number, a repository with a setup file.
fn parse_args(args: &[OsString]) -> Result<(PathBuf, u64, PathBuf), String> {
    if args.len() != 3 {
        let count = args.len();
        return Err(format!("expected 3 arguments, got {count}"));
    }
    let number = args[1]
        .to_str()
        .and_then(|text| text.parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| {
            let given = args[1].to_string_lossy();
            format!("issue number must be a positive integer: {given}")
        })?;
    Ok((PathBuf::from(&args[0]), number, PathBuf::from(&args[2])))
}

/// The command over an injected GitHub: returns the exit status.
pub fn run_with(gh: &mut dyn GitHub, args: Vec<OsString>) -> i32 {
    let (repo_dir, number, out_file) = match parse_args(&args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("ticket-export: {message}");
            eprint!("{USAGE}");
            return 2;
        }
    };
    // A refusal must leave no file behind, so a requirement from an earlier
    // export cannot be mistaken for this one.
    let _ = std::fs::remove_file(&out_file);
    let repo = match setup::tracker_repo(Path::new(&repo_dir)) {
        Ok(repo) => repo,
        Err(message) => {
            eprintln!("ticket-export: {message}");
            return 1;
        }
    };
    let document = match export(gh, &repo, number) {
        Ok(document) => document,
        Err(message) => {
            eprintln!("ticket-export: {message}");
            return 1;
        }
    };
    if let Err(e) = std::fs::write(&out_file, document) {
        let path = out_file.display();
        eprintln!("ticket-export: {path}: {e}");
        return 1;
    }
    let path = out_file.display();
    println!("ticket-export: wrote {path}");
    0
}

/// The command over the real `gh`.
pub fn run<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
    run_with(&mut GhCli::new(), args.into_iter().collect())
}
