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
        parsed.and_then(|issue| rest_issue(&issue, repository)).ok_or_else(|| "gh api printed something that is not an issue".to_string())
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
        let issues = pages.and_then(|pages| pages.iter().flatten().map(|issue| rest_issue(issue, repository)).collect::<Option<Vec<_>>>());
        issues.ok_or_else(|| "gh api printed something that is not a list of issues".to_string())
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

/// The record of a REST issue, which is an object with a numeric `number`; anything else is not one. The issue's
/// repository comes from its URL; one with no readable URL is taken to be in the repository asked.
fn rest_issue(issue: &Value, asked: &RepositoryName) -> Option<IssueRecord> {
    let fields = issue.as_object()?;
    let Some(Value::Num(number)) = fields.get("number") else {
        return None;
    };
    let text = |key: &str| fields.get(key).and_then(Value::as_str);
    Some(IssueRecord {
        closed: text("state") == Some("closed"),
        number: *number as u64,
        repository: text("html_url").and_then(parse_issue_url).map_or_else(|| asked.clone(), |url| url.reference.repository),
        title: text("title").unwrap_or_default().to_string(),
    })
}
