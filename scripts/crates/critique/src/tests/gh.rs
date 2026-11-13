//! The gh port against a fake `gh` program: the REST lists `check-tickets` reads (sub-issues and what blocks an
//! issue) and the comments `gh issue view` can carry, with what each answers when gh prints something else.

use crate::gh::{Gh, IssueComment, IssueLocator, IssueRequest, LinkedIssue, SystemGh};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// A folder holding a fake `gh` that prints `stdout` for any arguments; it also writes its arguments to `args`.
fn fake_gh(name: &str, stdout: &str) -> (PathBuf, SystemGh) {
    let dir = std::env::temp_dir().join(format!("critique-gh-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("out"), stdout).expect("out");
    let script = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{0}/args'\ncat '{0}/out'\n", dir.display());
    let gh = dir.join("gh");
    std::fs::write(&gh, script).expect("gh");
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut env = HashMap::new();
    env.insert("PATH".to_string(), format!("{}:/run/current-system/sw/bin:/usr/bin:/bin", dir.display()));
    (dir, SystemGh { env })
}

fn locator(dir: &PathBuf, repo: &str) -> IssueLocator {
    IssueLocator { cwd: dir.to_string_lossy().into_owned(), repo: repo.to_string(), number: 7.0 }
}

fn args(dir: &PathBuf) -> String {
    std::fs::read_to_string(dir.join("args")).expect("args")
}

#[test]
fn sub_issues_read_every_page_of_the_rest_list_on_the_host() {
    let pages = r#"[[{"number":2,"title":"T2","body":"b2","html_url":"https://x/2","labels":[{"name":"bug"}],"state":"open"}],[{"number":3,"html_url":"https://x/3","state":"weird"}]]"#;
    let (dir, gh) = fake_gh("sub", pages);
    let issues = gh.sub_issues(&locator(&dir, "github.com/o/r")).expect("sub-issues");
    assert_eq!(args(&dir), "api\n--hostname\ngithub.com\n--paginate\n--slurp\nrepos/o/r/issues/7/sub_issues?per_page=100\n");
    assert_eq!(issues.len(), 2);
    assert_eq!((issues[0].number, issues[0].title.as_str(), issues[0].body.as_str(), issues[0].url.as_str()), (2.0, "T2", "b2", "https://x/2"));
    assert_eq!(issues[0].labels, vec!["bug".to_string()]);
    assert_eq!(issues[0].state.as_deref(), Some("open"));
    assert!(issues[0].comments.is_empty());
    assert_eq!((issues[1].title.as_str(), issues[1].state.clone()), ("", None));
}

#[test]
fn blocked_by_reads_the_dependency_list() {
    let (dir, gh) = fake_gh("blocked", r#"[[{"number":4,"html_url":"https://x/4"}],[]]"#);
    let blockers = gh.blocked_by(&locator(&dir, "github.com/o/r")).expect("blocked by");
    assert!(args(&dir).ends_with("repos/o/r/issues/7/dependencies/blocked_by?per_page=100\n"));
    assert_eq!(blockers, vec![LinkedIssue { number: 4.0, url: "https://x/4".to_string() }]);
}

#[test]
fn a_rest_list_needs_a_host_owner_repo_and_a_list_of_issues() {
    let (dir, gh) = fake_gh("shape", "[[]]");
    assert_eq!(gh.sub_issues(&locator(&dir, "o/r")).unwrap_err(), "o/r is not a host/owner/repo repository");
    let not_list = "gh api printed something that is not a list of issues";
    for (name, stdout) in [("obj", "{}"), ("page", "[{}]"), ("item", r#"[[{"number":"2","html_url":"u"}]]"#), ("url", r#"[[{"number":2}]]"#), ("bad", "nope")] {
        let (dir, gh) = fake_gh(name, stdout);
        assert_eq!(gh.blocked_by(&locator(&dir, "github.com/o/r")).unwrap_err(), not_list, "{name}");
    }
    let (dir, gh) = fake_gh("num", r#"[[{"number":2,"html_url":"u"},{"number":3,"html_url":"v","title":5}]]"#);
    let issues = gh.sub_issues(&locator(&dir, "github.com/o/r")).expect("issues");
    assert_eq!(issues[1].title, "");
}

#[test]
fn an_issue_with_comments_reads_each_comment_and_skips_one_without_a_body() {
    let issue = r#"{"number":7,"title":"T","body":"B","url":"https://x/7","labels":[],"comments":[{"author":{"login":"ann"},"body":"first","createdAt":"2026-11-01T00:00:00Z"},{"body":"anon"},{"author":{"login":"bob"}}]}"#;
    let (dir, gh) = fake_gh("comments", issue);
    let read = gh.view_issue(&IssueRequest { locator: locator(&dir, "o/r"), comments: true }).expect("issue");
    assert!(args(&dir).ends_with("--json\nnumber,title,body,url,labels,comments\n"));
    assert_eq!(
        read.comments,
        vec![
            IssueComment { author: "ann".to_string(), created_at: "2026-11-01T00:00:00Z".to_string(), body: "first".to_string() },
            IssueComment { author: "unknown".to_string(), created_at: String::new(), body: "anon".to_string() },
        ]
    );
}

/// Only a ticket review takes the staged change; any other stage is refused before the rest of the input is read.
#[test]
fn only_a_ticket_review_takes_the_staged_change() {
    use crate::fields::Fields;
    use crate::staged_change::{staged_input, StagedInputError};
    let input = model_gateway::json::parse(r#"{"staged":true}"#).expect("input");
    let fields = Fields::of(&input, "").expect("fields");
    match staged_input(&fields, "spec") {
        Err(StagedInputError::Refused(error)) => {
            assert_eq!((error.code.as_str(), error.message.as_str()), ("invalid-input", "only ticket-review takes staged; spec reviews commits"));
        }
        _ => panic!("spec with staged was not refused"),
    }
}

#[test]
fn a_gateway_ledger_failure_reads_as_one_sentence() {
    assert_eq!(
        crate::gateway_ledger::gateway_ledger_warning("EACCES"),
        "the gateway ledger could not be written: EACCES; this run is recorded in critique's own ledger only"
    );
}

#[test]
fn a_comment_and_a_pinned_gateway_client_copy_across_as_they_are() {
    let comment = IssueComment { author: "ann".to_string(), created_at: "t".to_string(), body: "b".to_string() };
    let copied = crate::tracker_resolver::issue_comment(&comment);
    assert_eq!((copied.author.as_str(), copied.created_at.as_str(), copied.body.as_str()), ("ann", "t", "b"));
    let pinned = crate::routing::GatewayModel { client: "vibe".to_string(), model: "m".to_string() };
    let client = crate::triage_command::gateway_client(&pinned);
    assert_eq!((client.client.as_str(), client.model.as_str()), ("vibe", "m"));
}
