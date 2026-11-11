//! Tests at the two seams: the `GitHub` trait (the recorded `gh` answers,
//! served in memory) for the document and the refusals, and a tiny fake `gh`
//! script for the real client's request shape, deadline and failures.

use crate::{export, links, run, run_with, GhCli, GitHub, ReadError, PAGE};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const REPO: &str = "acme/kit";
const NUMBER: u64 = 531;

/// The requirement document the recorded fixtures must produce.
const EXPECTED: &str = "\
# Export a published issue to a requirement file (#531)

## Body

## Parent

#405

## What to build

A wrapper exports a published issue.

## Acceptance criteria

- [ ] Given a fixture issue, the wrapper writes the requirement file.
- [ ] The wrapper refuses a repository without a setup file.

## Scope

- Ticket key: `B2-11b`

## Acceptance criteria

- [ ] Given a fixture issue, the wrapper writes the requirement file.
- [ ] The wrapper refuses a repository without a setup file.

## Sub-issues

- #601 Read the issue (open)
- #602 Write the file (closed)

## Blocked by

- #452 Per-repository setup file (open)
";

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(relative)
}

/// One recorded `gh api` answer.
fn recorded(name: &str) -> String {
    std::fs::read_to_string(fixture(&format!("gh/{name}"))).unwrap()
}

fn issue_path() -> String {
    format!("repos/{REPO}/issues/{NUMBER}")
}

fn sub_path() -> String {
    format!("repos/{REPO}/issues/{NUMBER}/sub_issues?per_page=100")
}

fn blocked_path() -> String {
    format!("repos/{REPO}/issues/{NUMBER}/dependencies/blocked_by?per_page=100")
}

/// A GitHub that serves canned answers by path and records every read.
struct Fake {
    answers: Vec<(String, Result<String, ReadError>)>,
    calls: Vec<String>,
}

impl Fake {
    fn new(answers: Vec<(String, Result<String, ReadError>)>) -> Fake {
        Fake {
            answers,
            calls: Vec::new(),
        }
    }
}

impl GitHub for Fake {
    fn get(&mut self, path: &str) -> Result<String, ReadError> {
        self.calls.push(path.to_string());
        match self
            .answers
            .iter()
            .find(|(asked, _)| asked.as_str() == path)
        {
            Some((_, answer)) => answer.clone(),
            None => Err(ReadError::NotFound),
        }
    }
}

fn fake_of(
    issue: Result<String, ReadError>,
    sub_issues: Result<String, ReadError>,
    blocked_by: Result<String, ReadError>,
) -> Fake {
    Fake::new(vec![
        (issue_path(), issue),
        (sub_path(), sub_issues),
        (blocked_path(), blocked_by),
    ])
}

/// The recorded answers, as `gh` gave them.
fn fake() -> Fake {
    fake_of(
        Ok(recorded("issue.json")),
        Ok(recorded("sub-issues.json")),
        Ok(recorded("blocked-by.json")),
    )
}

fn temp_file(name: &str) -> PathBuf {
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("ticket-export-{pid}-{name}"));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn exports_the_recorded_issue() {
    let mut gh = fake();
    let document = export(&mut gh, REPO, NUMBER).unwrap();
    assert_eq!(document, EXPECTED);
    assert_eq!(gh.calls, vec![issue_path(), sub_path(), blocked_path()]);
}

#[test]
fn the_document_ends_with_one_newline() {
    let mut gh = fake();
    let document = export(&mut gh, REPO, NUMBER).unwrap();
    assert!(document.ends_with("(open)\n"));
    assert!(!document.ends_with("\n\n"));
}

#[test]
fn the_document_carries_no_author_labels_or_dates() {
    let mut gh = fake();
    let document = export(&mut gh, REPO, NUMBER).unwrap();
    for absent in [
        "octo",
        "type: feature",
        "2026-11-01",
        "2026-11-09",
        "created_at",
        "updated_at",
        "labels",
        "comments",
        "user",
    ] {
        assert!(
            !document.contains(absent),
            "{absent} leaked into the document"
        );
    }
}

#[test]
fn a_missing_issue_is_refused() {
    let mut gh = fake_of(
        Err(ReadError::NotFound),
        Ok(recorded("sub-issues.json")),
        Ok(recorded("blocked-by.json")),
    );
    assert_eq!(
        export(&mut gh, REPO, NUMBER),
        Err("issue #531 in acme/kit not found".to_string())
    );
    assert_eq!(gh.calls, vec![issue_path()]);
}

#[test]
fn missing_sub_issues_are_refused() {
    let mut gh = fake_of(
        Ok(recorded("issue.json")),
        Err(ReadError::NotFound),
        Ok(recorded("blocked-by.json")),
    );
    assert_eq!(
        export(&mut gh, REPO, NUMBER),
        Err("the sub-issues of issue #531 in acme/kit not found".to_string())
    );
    assert_eq!(gh.calls, vec![issue_path(), sub_path()]);
}

#[test]
fn missing_blockers_are_refused() {
    let mut gh = fake_of(
        Ok(recorded("issue.json")),
        Ok(recorded("sub-issues.json")),
        Err(ReadError::NotFound),
    );
    assert_eq!(
        export(&mut gh, REPO, NUMBER),
        Err("the blocked-by issues of issue #531 in acme/kit not found".to_string())
    );
    assert_eq!(gh.calls, vec![issue_path(), sub_path(), blocked_path()]);
}

#[test]
fn an_unreadable_issue_is_refused() {
    let mut gh = fake_of(
        Err(ReadError::Unreadable("gh: no such host".to_string())),
        Ok(recorded("sub-issues.json")),
        Ok(recorded("blocked-by.json")),
    );
    assert_eq!(
        export(&mut gh, REPO, NUMBER),
        Err("cannot read issue #531 in acme/kit: gh: no such host".to_string())
    );
    assert_eq!(gh.calls, vec![issue_path()]);
}

#[test]
fn a_closed_blocker_is_refused() {
    let mut gh = fake_of(
        Ok(recorded("issue.json")),
        Ok(recorded("sub-issues.json")),
        Ok(recorded("blocked-by-closed.json")),
    );
    assert_eq!(
        export(&mut gh, REPO, NUMBER),
        Err("issue #531 in acme/kit is blocked by closed issue #452".to_string())
    );
}

#[test]
fn malformed_json_is_refused() {
    let mut gh = fake_of(
        Ok("{not json".to_string()),
        Ok(recorded("sub-issues.json")),
        Ok(recorded("blocked-by.json")),
    );
    assert_eq!(
        export(&mut gh, REPO, NUMBER),
        Err("issue #531 in acme/kit: unexpected answer".to_string())
    );
}

#[test]
fn an_issue_without_a_title_or_state_is_refused() {
    let mut gh = fake_of(
        Ok(r#"{"number": 531, "title": "No state"}"#.to_string()),
        Ok(recorded("sub-issues.json")),
        Ok(recorded("blocked-by.json")),
    );
    assert_eq!(
        export(&mut gh, REPO, NUMBER),
        Err("issue #531 in acme/kit: unexpected answer".to_string())
    );
}

#[test]
fn a_list_that_is_not_an_array_is_refused() {
    let mut gh = fake_of(
        Ok(recorded("issue.json")),
        Ok(r#"{"number": 601}"#.to_string()),
        Ok(recorded("blocked-by.json")),
    );
    assert_eq!(
        export(&mut gh, REPO, NUMBER),
        Err("issue #531 in acme/kit: unexpected answer".to_string())
    );
}

#[test]
fn a_list_item_without_a_number_is_refused() {
    let mut gh = fake_of(
        Ok(recorded("issue.json")),
        Ok(r#"[{"title": "Read the issue", "state": "open"}]"#.to_string()),
        Ok(recorded("blocked-by.json")),
    );
    assert_eq!(
        export(&mut gh, REPO, NUMBER),
        Err("issue #531 in acme/kit: unexpected answer".to_string())
    );
}

#[test]
fn a_null_body_omits_the_body_block() {
    let mut gh = fake_of(
        Ok(r#"{"number": 531, "title": "No body", "state": "open", "body": null}"#.to_string()),
        Ok(recorded("sub-issues.json")),
        Ok(recorded("blocked-by.json")),
    );
    let document = export(&mut gh, REPO, NUMBER).unwrap();
    assert_eq!(
        document,
        "\
# No body (#531)

## Acceptance criteria

None

## Sub-issues

- #601 Read the issue (open)
- #602 Write the file (closed)

## Blocked by

- #452 Per-repository setup file (open)
"
    );
}

#[test]
fn a_body_without_a_criteria_section_says_none() {
    let mut gh = fake_of(
        Ok(
            r#"{"number": 531, "title": "Plain", "state": "open", "body": "Just text."}"#
                .to_string(),
        ),
        Ok(recorded("empty.json")),
        Ok(recorded("empty.json")),
    );
    let document = export(&mut gh, REPO, NUMBER).unwrap();
    assert_eq!(
        document,
        "\
# Plain (#531)

## Body

Just text.

## Acceptance criteria

None

## Sub-issues

None

## Blocked by

None
"
    );
}

#[test]
fn an_empty_criteria_section_says_none() {
    let mut gh = fake_of(
        Ok(r###"{"number": 531, "title": "Empty", "state": "open", "body": "## Acceptance criteria\n\n## Scope\n\n- x"}"###
            .to_string()),
        Ok(recorded("empty.json")),
        Ok(recorded("empty.json")),
    );
    let document = export(&mut gh, REPO, NUMBER).unwrap();
    assert!(document.contains("## Acceptance criteria\n\nNone\n"));
}

#[test]
fn a_crlf_body_is_normalised() {
    let mut gh = fake_of(
        Ok(r###"{"number": 531, "title": "Windows", "state": "open", "body": "## Acceptance criteria\r\n\r\n- [ ] It works.  \r\n"}"###
            .to_string()),
        Ok(recorded("empty.json")),
        Ok(recorded("empty.json")),
    );
    let document = export(&mut gh, REPO, NUMBER).unwrap();
    assert!(!document.contains('\r'));
    assert!(document.contains("## Acceptance criteria\n\n- [ ] It works.\n"));
}

#[test]
fn run_with_writes_the_requirement_file() {
    let mut gh = fake();
    let out = temp_file("written.md");
    let argv = vec![
        fixture("repo").into_os_string(),
        OsString::from("531"),
        out.clone().into_os_string(),
    ];
    assert_eq!(run_with(&mut gh, argv), 0);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), EXPECTED);
    assert_eq!(gh.calls, vec![issue_path(), sub_path(), blocked_path()]);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn too_few_arguments_are_usage() {
    let mut gh = fake();
    let argv = vec![fixture("repo").into_os_string(), OsString::from("531")];
    assert_eq!(run_with(&mut gh, argv), 2);
    assert!(gh.calls.is_empty());
}

#[test]
fn too_many_arguments_are_usage() {
    let mut gh = fake();
    let out = temp_file("too-many.md");
    let argv = vec![
        fixture("repo").into_os_string(),
        OsString::from("531"),
        out.clone().into_os_string(),
        OsString::from("extra"),
    ];
    assert_eq!(run_with(&mut gh, argv), 2);
    assert!(gh.calls.is_empty());
    assert!(!out.exists());
}

#[test]
fn a_number_that_is_not_a_positive_integer_is_usage() {
    for given in ["abc", "0", "-1", ""] {
        let mut gh = fake();
        let out = temp_file("bad-number.md");
        let argv = vec![
            fixture("repo").into_os_string(),
            OsString::from(given),
            out.clone().into_os_string(),
        ];
        assert_eq!(run_with(&mut gh, argv), 2, "{given}");
        assert!(gh.calls.is_empty());
        assert!(!out.exists());
    }
}

#[test]
fn a_repository_without_a_setup_file_is_refused() {
    let mut gh = fake();
    let out = temp_file("no-setup.md");
    let argv = vec![
        fixture("repo-without-setup").into_os_string(),
        OsString::from("531"),
        out.clone().into_os_string(),
    ];
    assert_eq!(run_with(&mut gh, argv), 1);
    assert!(gh.calls.is_empty());
    assert!(!out.exists());
}

#[test]
fn a_refused_export_writes_no_file() {
    let mut gh = fake_of(
        Err(ReadError::NotFound),
        Ok(recorded("sub-issues.json")),
        Ok(recorded("blocked-by.json")),
    );
    let out = temp_file("refused.md");
    let argv = vec![
        fixture("repo").into_os_string(),
        OsString::from("531"),
        out.clone().into_os_string(),
    ];
    assert_eq!(run_with(&mut gh, argv), 1);
    assert!(!out.exists());
}

#[test]
fn an_unwritable_out_file_is_refused() {
    let mut gh = fake();
    let out = fixture("repo");
    let argv = vec![
        fixture("repo").into_os_string(),
        OsString::from("531"),
        out.into_os_string(),
    ];
    assert_eq!(run_with(&mut gh, argv), 1);
}

#[test]
fn run_without_arguments_is_usage() {
    assert_eq!(run(Vec::<OsString>::new()), 2);
}

/// Held for a script's lifetime: writing an executable while another thread
/// forks makes that exec fail with "text file busy", so script tests run one
/// at a time.
static SCRIPTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A fake `gh` in a temporary directory, removed when the test ends.
struct Script {
    dir: PathBuf,
    path: PathBuf,
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Script {
    fn new(name: &str, body: &str) -> Script {
        let serial = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("ticket-export-script-{pid}-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gh");
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Script {
            dir,
            path,
            _serial: serial,
        }
    }

    fn client(&self, timeout: Duration) -> GhCli {
        GhCli::with(self.path.clone().into_os_string(), timeout)
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn gh_cli_asks_for_a_get_of_the_path() {
    let script = Script::new("args", "#!/bin/sh\nprintf '%s ' \"$@\"\n");
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.get("repos/acme/kit/issues/531"),
        Ok("api -X GET repos/acme/kit/issues/531 ".to_string())
    );
}

#[test]
fn gh_cli_returns_the_answer() {
    let script = Script::new("answer", "#!/bin/sh\nprintf '%s\\n' '{\"number\": 531}'\n");
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.get("repos/acme/kit/issues/531"),
        Ok("{\"number\": 531}\n".to_string())
    );
}

#[test]
fn gh_cli_reports_a_404_as_not_found() {
    let script = Script::new("404", "#!/bin/sh\necho 'HTTP 404: Not Found' >&2\nexit 1\n");
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.get("repos/acme/kit/issues/531"),
        Err(ReadError::NotFound)
    );
}

#[test]
fn gh_cli_reports_not_found_by_message() {
    let script = Script::new(
        "not-found",
        "#!/bin/sh\necho 'gh: Not Found (HTTP 404)' >&2\nexit 1\n",
    );
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.get("repos/acme/kit/issues/531"),
        Err(ReadError::NotFound)
    );
}

#[test]
fn gh_cli_reports_another_failure_as_unreadable() {
    let script = Script::new("boom", "#!/bin/sh\necho 'boom' >&2\nexit 1\n");
    let mut gh = script.client(Duration::from_secs(10));
    match gh.get("repos/acme/kit/issues/531") {
        Err(ReadError::Unreadable(message)) => {
            assert!(message.contains("boom"), "{message}");
            assert!(message.contains("repos/acme/kit/issues/531"), "{message}");
        }
        other => panic!("expected an unreadable answer, got {other:?}"),
    }
}

#[test]
fn gh_cli_kills_a_hung_call() {
    let script = Script::new("hung", "#!/bin/sh\nexec sleep 5\n");
    let mut gh = script.client(Duration::from_millis(100));
    match gh.get("repos/acme/kit/issues/531") {
        Err(ReadError::Unreadable(message)) => {
            assert!(message.contains("timed out after"), "{message}")
        }
        other => panic!("expected a timeout, got {other:?}"),
    }
}

#[test]
fn gh_cli_reports_a_missing_program() {
    let script = Script::new("missing", "#!/bin/sh\n");
    let missing = script.dir.join("no-such-gh");
    let mut gh = GhCli::with(missing.clone().into_os_string(), Duration::from_secs(10));
    match gh.get("repos/acme/kit/issues/531") {
        Err(ReadError::Unreadable(message)) => {
            assert!(message.contains("no-such-gh"), "{message}");
        }
        other => panic!("expected an unreadable answer, got {other:?}"),
    }
}

#[test]
fn default_is_the_real_gh_with_a_minute() {
    assert_eq!(
        format!("{:?}", GhCli::default()),
        format!("{:?}", GhCli::new())
    );
}

#[test]
fn gh_cli_does_not_call_a_403_with_404_in_its_text_not_found() {
    let script = Script::new(
        "403",
        "#!/bin/sh\necho 'HTTP 403 request 4041' >&2\nexit 1\n",
    );
    let mut gh = script.client(Duration::from_secs(10));
    assert!(matches!(
        gh.get("repos/acme/kit/issues/531"),
        Err(ReadError::Unreadable(_))
    ));
}

#[test]
fn a_full_page_of_links_is_refused_rather_than_truncated() {
    let item = r#"{"number": 1, "title": "t", "state": "open"}"#;
    let page = format!("[{}]", vec![item; PAGE].join(","));
    assert!(links(&page, 531, "acme/kit")
        .unwrap_err()
        .contains("more than one page"));
}

#[test]
fn a_refusal_removes_a_requirement_left_by_an_earlier_export() {
    let mut gh = fake();
    let out = temp_file("stale.md");
    std::fs::write(&out, "stale").unwrap();
    let argv = vec![
        fixture("repo-without-setup").into_os_string(),
        OsString::from("531"),
        out.clone().into_os_string(),
    ];
    assert_eq!(run_with(&mut gh, argv), 1);
    assert!(!out.exists());
}
