use crate::json::{parse, Value};
use crate::ports::{Issues, IssueRecord, Process};
use crate::spec::{parse_issue_url, RepositoryName};
use crate::text::first_line;

/// Reads issues through `gh api`. The REST API is used because `gh issue view` does not show sub-issues or
/// dependencies. Any failure is one line that says what gh reported.
pub struct GhCliIssues<P: Process> {
    pub process: P,
}

impl<P: Process> Issues for GhCliIssues<P> {
    fn issue(&self, repository: &RepositoryName, number: u64) -> Result<IssueRecord, String> {
        let parsed = self.api(repository, &format!("issues/{number}"), false)?;
        match parsed.filter(is_rest_issue) {
            Some(issue) => Ok(record(&issue, repository)),
            None => Err("gh api printed something that is not an issue".to_string()),
        }
    }

    fn sub_issues(&self, repository: &RepositoryName, number: u64) -> Result<Vec<IssueRecord>, String> {
        self.list(repository, &format!("issues/{number}/sub_issues"))
    }

    fn blocked_by(&self, repository: &RepositoryName, number: u64) -> Result<Vec<IssueRecord>, String> {
        self.list(repository, &format!("issues/{number}/dependencies/blocked_by"))
    }
}

impl<P: Process> GhCliIssues<P> {
    /// Every page of one of an issue's lists: `--paginate --slurp` prints one array per page inside an outer array.
    fn list(&self, repository: &RepositoryName, path: &str) -> Result<Vec<IssueRecord>, String> {
        let parsed = self.api(repository, &format!("{path}?per_page=100"), true)?;
        let pages = match parsed {
            Some(Value::Arr(pages)) => pages.into_iter().map(|page| if let Value::Arr(items) = page { Some(items) } else { None }).collect::<Option<Vec<_>>>(),
            _ => None,
        };
        let issues: Option<Vec<Value>> = pages.map(|pages| pages.into_iter().flatten().collect());
        match issues.filter(|issues| issues.iter().all(is_rest_issue)) {
            Some(issues) => Ok(issues.iter().map(|issue| record(issue, repository)).collect()),
            None => Err("gh api printed something that is not a list of issues".to_string()),
        }
    }

    fn api(&self, repository: &RepositoryName, path: &str, paginate: bool) -> Result<Option<Value>, String> {
        let endpoint = format!("repos/{}/{}/{path}", repository.owner, repository.repo);
        let mut args = vec!["api".to_string(), "--hostname".to_string(), repository.host.clone()];
        if paginate {
            args.extend(["--paginate".to_string(), "--slurp".to_string()]);
        }
        args.push(endpoint.clone());
        let result = self.process.exec("gh", &args)?;
        if result.exit_code != 0 {
            let detail = first_line(&result.stderr).or_else(|| first_line(&result.stdout)).unwrap_or_else(|| format!("exit code {}", result.exit_code));
            return Err(format!("gh api {endpoint} exited {}: {detail}", result.exit_code));
        }
        Ok(parse(&result.stdout).ok())
    }
}

fn is_rest_issue(value: &Value) -> bool {
    matches!(value.as_object().and_then(|issue| issue.get("number")), Some(Value::Num(_)))
}

/// The issue's repository comes from its URL; one with no readable URL is taken to be in the repository asked.
fn record(issue: &Value, asked: &RepositoryName) -> IssueRecord {
    let fields = issue.as_object();
    let text = |key: &str| fields.and_then(|fields| fields.get(key)).and_then(Value::as_str);
    let number = fields.and_then(|fields| fields.get("number")).map_or(0, |number| if let Value::Num(number) = number { *number as u64 } else { 0 });
    IssueRecord {
        closed: text("state") == Some("closed"),
        number,
        repository: text("html_url").and_then(parse_issue_url).map_or_else(|| asked.clone(), |url| url.reference.repository),
        title: text("title").unwrap_or_default().to_string(),
    }
}
