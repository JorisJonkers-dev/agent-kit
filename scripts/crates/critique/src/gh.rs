//! What the runner reads from GitHub through the `gh` CLI: an issue, its sub-issues and what blocks it. Every
//! failure is one line saying what gh reported. The node adapter's `adapters/gh`.

use model_gateway::adapters::fs::fail;
use model_gateway::json::{number_text, parse, Value};
use model_gateway::text::js_trim;
use std::collections::HashMap;
use std::process::{Command, Stdio};

/// One issue, named so gh never chooses the repository itself.
#[derive(Clone, Debug, PartialEq)]
pub struct IssueLocator {
    pub cwd: String,
    /// `[HOST/]OWNER/REPO`, passed to gh as `--repo`.
    pub repo: String,
    pub number: f64,
}

/// An issue to read, and whether its comments are read as well.
#[derive(Clone, Debug, PartialEq)]
pub struct IssueRequest {
    pub locator: IssueLocator,
    pub comments: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IssueComment {
    pub author: String,
    pub created_at: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GhIssue {
    pub number: f64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub labels: Vec<String>,
    /// Empty unless the request asked for comments.
    pub comments: Vec<IssueComment>,
    /// `open` or `closed`, where the read reports it (a sub-issue read does).
    pub state: Option<String>,
}

/// An issue another issue links to, such as a blocker; its web URL names its repository.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkedIssue {
    pub number: f64,
    pub url: String,
}

/// What the runner reads from GitHub.
pub trait Gh: Send + Sync {
    /// Reads one issue; an `Err` when gh fails or prints something that is not an issue.
    fn view_issue(&self, request: &IssueRequest) -> Result<GhIssue, String>;
    /// An issue's sub-issues, in GitHub's order, without their comments.
    fn sub_issues(&self, locator: &IssueLocator) -> Result<Vec<GhIssue>, String>;
    /// The issues GitHub's native dependencies say block this one.
    fn blocked_by(&self, locator: &IssueLocator) -> Result<Vec<LinkedIssue>, String>;
}

/// gh in the environment it was given (PATH decides which gh).
pub struct SystemGh {
    pub env: HashMap<String, String>,
}

impl SystemGh {
    /// Runs gh in `cwd`; any exit code but 0 fails with `<what> exited <code>: <first line gh said>`.
    fn run(&self, cwd: &str, args: &[&str], what: &str) -> Result<String, String> {
        let output = Command::new("gh")
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("spawn gh {}", fail(&error, "spawn", None).code))?;
        // Node reports a command a signal ended as 124.
        let code = output.status.code().unwrap_or(124);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if code == 0 {
            return Ok(stdout);
        }
        let detail = first_line(&String::from_utf8_lossy(&output.stderr)).or_else(|| first_line(&stdout)).unwrap_or_else(|| format!("exit code {code}"));
        Err(format!("{what} exited {code}: {detail}"))
    }

    /// Every page of one of an issue's REST lists, on the locator's host: `gh issue view` shows neither sub-issues
    /// nor dependencies.
    fn rest_list(&self, locator: &IssueLocator, list: &str) -> Result<Vec<Value>, String> {
        let parts: Vec<&str> = locator.repo.split('/').collect();
        let [host, owner, repo] = parts[..] else {
            return Err(format!("{} is not a host/owner/repo repository", locator.repo));
        };
        let path = format!("repos/{owner}/{repo}/issues/{}/{list}?per_page=100", number_text(locator.number));
        let stdout = self.run(&locator.cwd, &["api", "--hostname", host, "--paginate", "--slurp", &path], "gh api")?;
        read_pages(&stdout).ok_or_else(|| "gh api printed something that is not a list of issues".to_string())
    }
}

impl Gh for SystemGh {
    fn view_issue(&self, request: &IssueRequest) -> Result<GhIssue, String> {
        let fields = if request.comments { "number,title,body,url,labels,comments" } else { "number,title,body,url,labels" };
        let number = number_text(request.locator.number);
        let stdout = self.run(&request.locator.cwd, &["issue", "view", &number, "--repo", &request.locator.repo, "--json", fields], "gh issue view")?;
        read_issue(&stdout).ok_or_else(|| "gh issue view printed something that is not an issue".to_string())
    }

    fn sub_issues(&self, locator: &IssueLocator) -> Result<Vec<GhIssue>, String> {
        Ok(self.rest_list(locator, "sub_issues")?.iter().map(sub_issue).collect())
    }

    fn blocked_by(&self, locator: &IssueLocator) -> Result<Vec<LinkedIssue>, String> {
        Ok(self.rest_list(locator, "dependencies/blocked_by")?.iter().map(|issue| LinkedIssue { number: number_of(issue), url: string_of(issue, "html_url") }).collect())
    }
}

/// The first line of the text that holds something, trimmed.
fn first_line(text: &str) -> Option<String> {
    text.split('\n').map(js_trim).find(|line| !line.is_empty()).map(str::to_string)
}

fn member<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|object| object.get(key))
}

fn string_of(value: &Value, key: &str) -> String {
    member(value, key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn number_of(value: &Value) -> f64 {
    match member(value, "number") {
        Some(Value::Num(number)) => *number,
        _ => 0.0,
    }
}

/// The names of the labels in a list; anything that is no label with a name is skipped.
fn label_names(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Arr(labels)) => labels.iter().filter_map(|label| member(label, "name").and_then(Value::as_str)).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// `--paginate --slurp` prints one array per page inside an outer array; every issue needs a number and a web URL.
fn read_pages(stdout: &str) -> Option<Vec<Value>> {
    let Value::Arr(pages) = parse(stdout).ok()? else {
        return None;
    };
    let mut issues = Vec::new();
    for page in pages {
        let Value::Arr(items) = page else {
            return None;
        };
        issues.extend(items);
    }
    let rest_issue = |issue: &Value| matches!(member(issue, "number"), Some(Value::Num(_))) && matches!(member(issue, "html_url"), Some(Value::Str(_)));
    issues.iter().all(rest_issue).then_some(issues)
}

fn sub_issue(issue: &Value) -> GhIssue {
    GhIssue {
        body: string_of(issue, "body"),
        comments: Vec::new(),
        labels: label_names(member(issue, "labels")),
        number: number_of(issue),
        state: member(issue, "state").and_then(Value::as_str).filter(|state| matches!(*state, "open" | "closed")).map(str::to_string),
        title: string_of(issue, "title"),
        url: string_of(issue, "html_url"),
    }
}

/// `gh issue view --json` output, as one issue; `None` when it is not one.
fn read_issue(stdout: &str) -> Option<GhIssue> {
    let parsed = parse(stdout).ok()?;
    parsed.as_object()?;
    let (Some(Value::Num(number)), Some(Value::Str(title)), Some(Value::Str(body)), Some(Value::Str(url))) =
        (member(&parsed, "number"), member(&parsed, "title"), member(&parsed, "body"), member(&parsed, "url"))
    else {
        return None;
    };
    let comments = match member(&parsed, "comments") {
        Some(Value::Arr(comments)) => comments.iter().filter_map(read_comment).collect(),
        _ => Vec::new(),
    };
    Some(GhIssue {
        body: body.clone(),
        comments,
        labels: label_names(member(&parsed, "labels")),
        number: *number,
        state: None,
        title: title.clone(),
        url: url.clone(),
    })
}

fn read_comment(comment: &Value) -> Option<IssueComment> {
    let body = member(comment, "body").and_then(Value::as_str)?;
    let author = member(comment, "author").and_then(|author| member(author, "login")).and_then(Value::as_str).unwrap_or("unknown");
    Some(IssueComment { author: author.to_string(), body: body.to_string(), created_at: string_of(comment, "createdAt") })
}
