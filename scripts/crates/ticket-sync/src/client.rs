//! The GitHub seam. `Client` is the interface the planner and the executor
//! use; tests fake it. `GhClient` is the real one, over the `gh` CLI, which
//! itself sits on a `Runner` so its request shapes are testable too.

use crate::model::Result;
use serde::Deserialize;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub id: u64,
    pub title: String,
    pub body: String,
    pub open: bool,
}

/// A linked issue: its number and its node-independent REST id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Link {
    pub number: u64,
    pub id: u64,
}

pub trait Client {
    fn issue(&mut self, number: u64) -> Result<Issue>;
    fn create(&mut self, title: &str, body: &str, labels: &[String]) -> Result<Issue>;
    fn edit(&mut self, number: u64, title: Option<&str>, body: Option<&str>) -> Result<()>;
    fn sub_issues(&mut self, parent: u64) -> Result<Vec<Link>>;
    fn add_sub_issue(&mut self, parent: u64, child_id: u64) -> Result<()>;
    fn remove_sub_issue(&mut self, parent: u64, child_id: u64) -> Result<()>;
    fn blocked_by(&mut self, number: u64) -> Result<Vec<Link>>;
    fn add_blocked_by(&mut self, number: u64, blocker_id: u64) -> Result<()>;
    fn remove_blocked_by(&mut self, number: u64, blocker_id: u64) -> Result<()>;
    fn close(&mut self, number: u64, comment: &str) -> Result<()>;
}

/// Runs one `gh` invocation and returns its stdout.
pub trait Runner {
    fn run(&mut self, args: &[String], stdin: Option<&str>) -> Result<String>;
}

pub struct ProcessRunner {
    program: String,
}

impl ProcessRunner {
    pub fn new(program: &str) -> Self {
        ProcessRunner { program: program.to_string() }
    }
}

impl Runner for ProcessRunner {
    fn run(&mut self, args: &[String], stdin: Option<&str>) -> Result<String> {
        let mut child = Command::new(&self.program)
            .args(args)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", self.program))?;
        if let Some(input) = stdin {
            child.stdin.take().expect("piped").write_all(input.as_bytes()).map_err(|e| e.to_string())?;
        }
        // Waiting on a child that was just spawned does not fail in practice.
        let out = child.wait_with_output().expect("wait for gh");
        if !out.status.success() {
            return Err(format!("{} {}: {}", self.program, args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

#[derive(Deserialize)]
struct RawIssue {
    number: u64,
    id: u64,
    title: String,
    body: Option<String>,
    state: String,
}

#[derive(Deserialize)]
struct RawLink {
    number: u64,
    id: u64,
}

impl From<RawIssue> for Issue {
    fn from(r: RawIssue) -> Issue {
        Issue { number: r.number, id: r.id, title: r.title, body: r.body.unwrap_or_default(), open: r.state == "open" }
    }
}

pub struct GhClient {
    repo: String,
    runner: Box<dyn Runner>,
}

impl GhClient {
    pub fn new(repo: &str, runner: Box<dyn Runner>) -> Self {
        GhClient { repo: repo.to_string(), runner }
    }

    fn call(&mut self, method: &str, path: &str, json: Option<serde_json::Value>) -> Result<String> {
        let mut args: Vec<String> = vec!["api".into(), "-X".into(), method.into(), format!("repos/{}/{path}", self.repo)];
        let input = json.map(|v| v.to_string());
        if input.is_some() {
            args.extend(["--input".to_string(), "-".to_string()]);
        }
        self.runner.run(&args, input.as_deref())
    }

    /// A paginated list. `gh api --paginate` prints one JSON array per page,
    /// back to back, so the output is read as a stream of arrays.
    fn list(&mut self, path: &str) -> Result<Vec<Link>> {
        let args: Vec<String> = vec!["api".into(), "--paginate".into(), format!("repos/{}/{path}?per_page=100", self.repo)];
        let out = self.runner.run(&args, None)?;
        let mut links = Vec::new();
        for page in serde_json::Deserializer::from_str(&out).into_iter::<Vec<RawLink>>() {
            for l in page.map_err(|e| format!("{path}: {e}"))? {
                links.push(Link { number: l.number, id: l.id });
            }
        }
        Ok(links)
    }

    fn parse_issue(out: &str) -> Result<Issue> {
        serde_json::from_str::<RawIssue>(out).map(Issue::from).map_err(|e| format!("issue response: {e}"))
    }
}

impl Client for GhClient {
    fn issue(&mut self, number: u64) -> Result<Issue> {
        let out = self.call("GET", &format!("issues/{number}"), None)?;
        Self::parse_issue(&out)
    }

    fn create(&mut self, title: &str, body: &str, labels: &[String]) -> Result<Issue> {
        let out = self.call("POST", "issues", Some(serde_json::json!({"title": title, "body": body, "labels": labels})))?;
        Self::parse_issue(&out)
    }

    fn edit(&mut self, number: u64, title: Option<&str>, body: Option<&str>) -> Result<()> {
        let mut patch = serde_json::Map::new();
        if let Some(t) = title {
            patch.insert("title".into(), t.into());
        }
        if let Some(b) = body {
            patch.insert("body".into(), b.into());
        }
        self.call("PATCH", &format!("issues/{number}"), Some(patch.into())).map(drop)
    }

    fn sub_issues(&mut self, parent: u64) -> Result<Vec<Link>> {
        self.list(&format!("issues/{parent}/sub_issues"))
    }

    fn add_sub_issue(&mut self, parent: u64, child_id: u64) -> Result<()> {
        self.call("POST", &format!("issues/{parent}/sub_issues"), Some(serde_json::json!({"sub_issue_id": child_id}))).map(drop)
    }

    fn remove_sub_issue(&mut self, parent: u64, child_id: u64) -> Result<()> {
        self.call("DELETE", &format!("issues/{parent}/sub_issue"), Some(serde_json::json!({"sub_issue_id": child_id}))).map(drop)
    }

    fn blocked_by(&mut self, number: u64) -> Result<Vec<Link>> {
        self.list(&format!("issues/{number}/dependencies/blocked_by"))
    }

    fn add_blocked_by(&mut self, number: u64, blocker_id: u64) -> Result<()> {
        self.call("POST", &format!("issues/{number}/dependencies/blocked_by"), Some(serde_json::json!({"issue_id": blocker_id}))).map(drop)
    }

    fn remove_blocked_by(&mut self, number: u64, blocker_id: u64) -> Result<()> {
        self.call("DELETE", &format!("issues/{number}/dependencies/blocked_by/{blocker_id}"), None).map(drop)
    }

    fn close(&mut self, number: u64, comment: &str) -> Result<()> {
        self.call("PATCH", &format!("issues/{number}"), Some(serde_json::json!({"state": "closed", "state_reason": "not_planned"})))?;
        self.call("POST", &format!("issues/{number}/comments"), Some(serde_json::json!({"body": comment}))).map(drop)
    }
}
