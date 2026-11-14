//! ticket-publish: publish drafted tickets to the tracker the repository's own
//! setup file names.
//!
//! Nothing is sent until the setup file and the whole tickets file are valid,
//! so a refusal creates no issue. Each ticket is one `gh issue create`; a
//! failure part way stops the run rather than retrying or carrying on.

pub mod setup;

#[cfg(test)]
mod tests;

use serde_json::Value;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: ticket-publish check <REPO_DIR>\n       \
                     ticket-publish publish <REPO_DIR> <TICKETS_FILE>\n";

/// How often a running `gh` is asked whether it has finished.
const POLL: Duration = Duration::from_millis(10);

/// The one thing this tool may do to the tracker: create an issue and get its
/// URL back.
pub trait Tracker {
    fn create(
        &mut self,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<String, String>;
}

/// The real seam: one `gh issue create` per ticket, with a deadline.
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

impl Tracker for GhCli {
    fn create(
        &mut self,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<String, String> {
        let mut command = Command::new(&self.program);
        command
            .arg("issue")
            .arg("create")
            .arg("--repo")
            .arg(repo)
            .arg("--title")
            .arg(title)
            .arg("--body-file")
            .arg("-");
        for label in labels {
            command.arg("--label").arg(label);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                let program = self.program.to_string_lossy();
                format!("{program}: {e}")
            })?;
        // Both pipes are read on their own thread, so a large answer cannot
        // fill a pipe buffer and deadlock the wait below.
        let out_reader = child.stdout.take().map(reader);
        let err_reader = child.stderr.take().map(reader);
        // A child that exits before reading the body closes the pipe; its
        // exit status is the answer that matters.
        let _ = child.stdin.take().map(|mut stdin| stdin.write_all(body.as_bytes()));
        let status = match wait(&mut child, self.timeout) {
            Ok(status) => status,
            Err(message) => return Err(format!("gh issue create --repo {repo}: {message}")),
        };
        // Both pipes were asked for, so both readers are there.
        let out = out_reader.map(|handle| handle.join().unwrap_or_default()).unwrap_or_default();
        let err = err_reader.map(|handle| handle.join().unwrap_or_default()).unwrap_or_default();
        if !status.success() {
            let message = err.trim();
            return Err(format!("gh issue create --repo {repo}: {message}"));
        }
        match out.lines().rev().find(|line| !line.trim().is_empty()) {
            Some(url) => Ok(url.trim().to_string()),
            None => Err(format!(
                "gh issue create --repo {repo}: no issue URL in the answer"
            )),
        }
    }
}

/// Drain one pipe to a string on its own thread. A read error leaves what was
/// read so far, which then fails the URL check rather than the read.
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
            let seconds = timeout.as_secs_f64();
            return Err(format!("timed out after {seconds}s"));
        }
        std::thread::sleep(POLL);
    }
}

/// One drafted ticket, as the tickets file carries it.
#[derive(Debug, PartialEq, Eq)]
struct Ticket {
    title: String,
    body: String,
    labels: Vec<String>,
}

/// The tickets file's text, or the refusal that says why it cannot be read.
fn read_tickets(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    String::from_utf8(bytes).map_err(|e| format!("{}: not UTF-8: {e}", path.display()))
}

/// The tickets the file holds, or the refusal that says why it is not that
/// shape. Every ticket is checked before any is published.
fn tickets(text: &str) -> Result<Vec<Ticket>, String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| format!("the tickets file is not valid JSON: {e}"))?;
    let list = value
        .get("tickets")
        .and_then(Value::as_array)
        .ok_or_else(|| "the tickets file must be an object with a tickets list".to_string())?;
    if list.is_empty() {
        return Err("the tickets file holds no tickets".to_string());
    }
    let mut found = Vec::new();
    for (index, item) in list.iter().enumerate() {
        found.push(ticket(index, item)?);
    }
    Ok(found)
}

fn ticket(index: usize, item: &Value) -> Result<Ticket, String> {
    let title = text_field(index, item, "title")?;
    let body = text_field(index, item, "body")?;
    let labels = item
        .get("labels")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("tickets[{index}].labels must be a list of names"))?;
    let mut names = Vec::new();
    for label in labels {
        match label.as_str() {
            Some(name) => names.push(name.to_string()),
            None => return Err(format!("tickets[{index}].labels must be a list of names")),
        }
    }
    Ok(Ticket {
        title,
        body,
        labels: names,
    })
}

fn text_field(index: usize, item: &Value, field: &str) -> Result<String, String> {
    match item.get(field).and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => Ok(text.to_string()),
        _ => Err(format!(
            "tickets[{index}].{field} must be a non-empty string"
        )),
    }
}

/// The first ticket label the setup file does not list, with the ticket index.
fn unknown_label(tickets: &[Ticket], vocabulary: &[String]) -> Option<String> {
    for (index, ticket) in tickets.iter().enumerate() {
        for label in &ticket.labels {
            if !vocabulary.iter().any(|known| known == label) {
                return Some(format!(
                    "tickets[{index}]: label {label:?} is not in docs/agents/setup.yaml labels"
                ));
            }
        }
    }
    None
}

/// A refusal before anything is sent: the reason on stderr, exit 3.
fn refuse(message: &str) -> i32 {
    eprintln!("ticket-publish: {message}");
    3
}

/// `check`: the setup file is valid enough to publish. Nothing is written or
/// sent anywhere.
fn check(repo_dir: &Path) -> i32 {
    match ticket_export::setup::tracker_repo(repo_dir).and_then(|_| setup::labels(repo_dir)) {
        Ok(_) => {
            println!("ok");
            0
        }
        Err(message) => refuse(&message),
    }
}

/// `publish`: validate everything, then create one issue per ticket in order.
fn publish(tracker: &mut dyn Tracker, repo_dir: &Path, tickets_file: &Path) -> i32 {
    let repo = match ticket_export::setup::tracker_repo(repo_dir) {
        Ok(repo) => repo,
        Err(message) => return refuse(&message),
    };
    let vocabulary = match setup::labels(repo_dir) {
        Ok(labels) => labels,
        Err(message) => return refuse(&message),
    };
    let text = match read_tickets(tickets_file) {
        Ok(text) => text,
        Err(message) => return refuse(&message),
    };
    let drafted = match tickets(&text) {
        Ok(drafted) => drafted,
        Err(message) => return refuse(&message),
    };
    if let Some(message) = unknown_label(&drafted, &vocabulary) {
        return refuse(&message);
    }
    for ticket in &drafted {
        match tracker.create(&repo, &ticket.title, &ticket.body, &ticket.labels) {
            // Each URL is printed as it arrives, so a failure part way leaves
            // the ones already created on stdout.
            Ok(url) => println!("{url}"),
            Err(message) => {
                eprintln!("ticket-publish: {message}");
                return 1;
            }
        }
    }
    0
}

/// What the command line asked for.
enum Args {
    Check {
        repo_dir: PathBuf,
    },
    Publish {
        repo_dir: PathBuf,
        tickets_file: PathBuf,
    },
}

/// `check <REPO_DIR>` or `publish <REPO_DIR> <TICKETS_FILE>`: exactly the
/// arguments the subcommand takes.
fn parse_args(args: &[OsString]) -> Result<Args, String> {
    let (check, rest) = match args.split_first() {
        Some((first, rest)) => match first.to_str() {
            Some("check") => (true, rest),
            Some("publish") => (false, rest),
            _ => {
                let given = first.to_string_lossy();
                return Err(format!("unknown subcommand: {given}"));
            }
        },
        None => return Err("expected a subcommand: check or publish".to_string()),
    };
    let wanted = if check { 1 } else { 2 };
    if rest.len() != wanted {
        let count = rest.len();
        return Err(format!(
            "expected {wanted} arguments after the subcommand, got {count}"
        ));
    }
    let repo_dir = PathBuf::from(&rest[0]);
    if check {
        return Ok(Args::Check { repo_dir });
    }
    Ok(Args::Publish {
        repo_dir,
        tickets_file: PathBuf::from(&rest[1]),
    })
}

/// The command over an injected tracker: returns the exit status.
pub fn run_with(tracker: &mut dyn Tracker, args: Vec<OsString>) -> i32 {
    match parse_args(&args) {
        Ok(Args::Check { repo_dir }) => check(&repo_dir),
        Ok(Args::Publish {
            repo_dir,
            tickets_file,
        }) => publish(tracker, &repo_dir, &tickets_file),
        Err(message) => {
            eprintln!("ticket-publish: {message}");
            eprint!("{USAGE}");
            2
        }
    }
}

/// The command over the real `gh`.
pub fn run<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
    run_with(&mut GhCli::new(), args.into_iter().collect())
}
