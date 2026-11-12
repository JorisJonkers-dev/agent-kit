//! Tests at the public seam: `run_with` over an injected tracker for the
//! refusals, the order and the partial failure, and a tiny fake `gh` script
//! for the real client's request shape, deadline and failures.

use crate::{run, run_with, tickets, unknown_label, GhCli, Tracker};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const REPO: &str = "acme/kit";

/// A setup file the tool accepts, with the fixture's label vocabulary.
const SETUP: &str = "version: 1\n\
    tracker: {kind: github, repo: acme/kit}\n\
    labels: [\"type: feature\", needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix]\n\
    triage_labels: {needs-triage: needs-triage, needs-info: needs-info, ready-for-agent: ready-for-agent, ready-for-human: ready-for-human, wontfix: wontfix}\n\
    flaky_jobs: []\n";

const TICKETS: &str = r#"{"tickets":[
  {"title":"First","body":"Body one.","labels":["needs-triage"]},
  {"title":"Second","body":"Body two.","labels":[]}
]}"#;

const THREE: &str = r#"{"tickets":[
  {"title":"One","body":"B1","labels":[]},
  {"title":"Two","body":"B2","labels":[]},
  {"title":"Three","body":"B3","labels":[]}
]}"#;

const UNKNOWN: &str = r#"{"tickets":[
  {"title":"One","body":"B1","labels":["needs-triage"]},
  {"title":"Two","body":"B2","labels":["ready-for-agent","nope"]}
]}"#;

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(relative)
}

/// A path as the command line carries it.
fn os(path: &Path) -> OsString {
    path.to_path_buf().into_os_string()
}

/// A tracker that answers from a script and records every call.
struct Fake {
    answers: Vec<Result<String, String>>,
    calls: Vec<(String, String, String, Vec<String>)>,
}

impl Fake {
    fn new(answers: Vec<Result<String, String>>) -> Fake {
        Fake {
            answers,
            calls: Vec::new(),
        }
    }

    /// A tracker that creates `count` issues, numbered from one.
    fn ok(count: usize) -> Fake {
        Fake::new(
            (1..=count)
                .map(|n| Ok(format!("https://github.com/{REPO}/issues/{n}")))
                .collect(),
        )
    }
}

impl Tracker for Fake {
    fn create(
        &mut self,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<String, String> {
        self.calls.push((
            repo.to_string(),
            title.to_string(),
            body.to_string(),
            labels.to_vec(),
        ));
        if self.answers.is_empty() {
            return Err("the fake has no answer left".to_string());
        }
        self.answers.remove(0)
    }
}

/// A throwaway repository with a tickets file, removed when the test ends.
struct Temp {
    dir: PathBuf,
}

impl Temp {
    fn new(test: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ticket-publish-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs/agents")).expect("create the temporary repository");
        Self { dir }
    }

    fn setup(self, text: &str) -> Self {
        std::fs::write(self.dir.join("docs/agents/setup.yaml"), text).expect("write setup.yaml");
        self
    }

    fn tickets(self, text: &str) -> Self {
        std::fs::write(self.dir.join("tickets.json"), text).expect("write tickets.json");
        self
    }

    fn tickets_bytes(self, bytes: &[u8]) -> Self {
        std::fs::write(self.dir.join("tickets.json"), bytes).expect("write tickets.json");
        self
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn tickets_path(&self) -> PathBuf {
        self.dir.join("tickets.json")
    }

    fn publish_args(&self) -> Vec<OsString> {
        vec![
            OsString::from("publish"),
            os(self.path()),
            os(&self.tickets_path()),
        ]
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn publishes_every_ticket_in_order() {
    let repo = Temp::new("publish").setup(SETUP).tickets(TICKETS);
    let mut tracker = Fake::ok(2);
    assert_eq!(run_with(&mut tracker, repo.publish_args()), 0);
    assert_eq!(
        tracker.calls,
        vec![
            (
                REPO.to_string(),
                "First".to_string(),
                "Body one.".to_string(),
                vec!["needs-triage".to_string()],
            ),
            (
                REPO.to_string(),
                "Second".to_string(),
                "Body two.".to_string(),
                Vec::new(),
            ),
        ]
    );
}

#[test]
fn a_failure_part_way_keeps_the_urls_already_created_and_stops() {
    let repo = Temp::new("partial").setup(SETUP).tickets(THREE);
    let mut tracker = Fake::new(vec![
        Ok("https://github.com/acme/kit/issues/1".to_string()),
        Err("gh issue create --repo acme/kit: boom".to_string()),
    ]);
    assert_eq!(run_with(&mut tracker, repo.publish_args()), 1);
    assert_eq!(tracker.calls.len(), 2, "the third ticket is never created");
    assert_eq!(tracker.calls[1].1, "Two");
}

#[test]
fn check_accepts_a_valid_setup_file() {
    let mut tracker = Fake::new(Vec::new());
    let argv = vec![OsString::from("check"), os(&fixture("repo"))];
    assert_eq!(run_with(&mut tracker, argv), 0);
    assert!(tracker.calls.is_empty());
}

#[test]
fn check_refuses_a_repository_without_a_setup_file() {
    let mut tracker = Fake::new(Vec::new());
    let argv = vec![OsString::from("check"), os(&fixture("repo-without-setup"))];
    assert_eq!(run_with(&mut tracker, argv), 3);
    assert!(tracker.calls.is_empty());
}

#[test]
fn check_refuses_a_setup_file_that_is_not_valid() {
    for (name, text) in [
        (
            "version",
            "version: 2\ntracker: {kind: github, repo: acme/kit}\nlabels: [a]\n",
        ),
        (
            "tracker",
            "version: 1\ntracker: {kind: gitlab, repo: acme/kit}\nlabels: [a]\n",
        ),
        (
            "labels",
            "version: 1\ntracker: {kind: github, repo: acme/kit}\nlabels: []\n",
        ),
        (
            "no-labels",
            "version: 1\ntracker: {kind: github, repo: acme/kit}\n",
        ),
    ] {
        let repo = Temp::new(&format!("check-invalid-{name}")).setup(text);
        let mut tracker = Fake::new(Vec::new());
        let argv = vec![OsString::from("check"), os(repo.path())];
        assert_eq!(run_with(&mut tracker, argv), 3, "{name}");
        assert!(tracker.calls.is_empty(), "{name}");
    }
}

#[test]
fn publish_refuses_a_repository_without_a_setup_file() {
    let repo = Temp::new("publish-no-setup").tickets(TICKETS);
    let mut tracker = Fake::ok(2);
    assert_eq!(run_with(&mut tracker, repo.publish_args()), 3);
    assert!(tracker.calls.is_empty());
}

#[test]
fn publish_refuses_an_invalid_setup_file() {
    let repo = Temp::new("publish-bad-setup")
        .setup("version: 1\ntracker: {kind: github, repo: acme/kit}\nlabels: []\n")
        .tickets(TICKETS);
    let mut tracker = Fake::ok(2);
    assert_eq!(run_with(&mut tracker, repo.publish_args()), 3);
    assert!(tracker.calls.is_empty());
}

#[test]
fn publish_refuses_a_missing_tickets_file() {
    let repo = Temp::new("publish-no-tickets").setup(SETUP);
    let mut tracker = Fake::ok(2);
    assert_eq!(run_with(&mut tracker, repo.publish_args()), 3);
    assert!(tracker.calls.is_empty());
}

#[test]
fn publish_refuses_a_tickets_file_that_is_not_utf8() {
    let repo = Temp::new("publish-not-utf8")
        .setup(SETUP)
        .tickets_bytes(&[b'{', 0xff, 0xfe]);
    let mut tracker = Fake::ok(2);
    assert_eq!(run_with(&mut tracker, repo.publish_args()), 3);
    assert!(tracker.calls.is_empty());
}

#[test]
fn publish_refuses_a_tickets_file_that_is_not_that_shape() {
    for (name, text) in [
        ("not-json", "{not json"),
        ("not-an-object", "[]"),
        ("no-list", r#"{"tickets":{}}"#),
        ("empty", r#"{"tickets":[]}"#),
        ("no-title", r#"{"tickets":[{"body":"B","labels":[]}]}"#),
        ("no-labels", r#"{"tickets":[{"title":"T","body":"B"}]}"#),
    ] {
        let repo = Temp::new(&format!("publish-shape-{name}"))
            .setup(SETUP)
            .tickets(text);
        let mut tracker = Fake::ok(2);
        assert_eq!(run_with(&mut tracker, repo.publish_args()), 3, "{name}");
        assert!(tracker.calls.is_empty(), "{name}");
    }
}

#[test]
fn publish_refuses_a_label_the_setup_file_does_not_list() {
    let repo = Temp::new("publish-unknown-label")
        .setup(SETUP)
        .tickets(UNKNOWN);
    let mut tracker = Fake::ok(2);
    assert_eq!(run_with(&mut tracker, repo.publish_args()), 3);
    assert!(tracker.calls.is_empty());
}

#[test]
fn an_unknown_label_names_the_ticket_and_the_label() {
    let drafted = tickets(UNKNOWN).expect("the tickets file is valid");
    let vocabulary = vec!["needs-triage".to_string(), "ready-for-agent".to_string()];
    assert_eq!(
        unknown_label(&drafted, &vocabulary),
        Some("tickets[1]: label \"nope\" is not in docs/agents/setup.yaml labels".to_string())
    );
}

#[test]
fn a_known_label_vocabulary_leaves_nothing_unknown() {
    let drafted = tickets(TICKETS).expect("the tickets file is valid");
    let vocabulary = vec!["needs-triage".to_string()];
    assert_eq!(unknown_label(&drafted, &vocabulary), None);
}

#[test]
fn a_tickets_file_that_is_not_json_is_refused() {
    let err = tickets("{not json").expect_err("the file must be refused");
    assert!(
        err.starts_with("the tickets file is not valid JSON: "),
        "{err}"
    );
}

#[test]
fn a_tickets_file_without_a_tickets_list_is_refused() {
    for text in ["[]", "{}", r#"{"tickets":{}}"#, r#"{"tickets":"one"}"#] {
        assert_eq!(
            tickets(text),
            Err("the tickets file must be an object with a tickets list".to_string()),
            "{text}"
        );
    }
}

#[test]
fn an_empty_tickets_list_is_refused() {
    assert_eq!(
        tickets(r#"{"tickets":[]}"#),
        Err("the tickets file holds no tickets".to_string())
    );
}

#[test]
fn a_ticket_without_a_title_a_body_or_a_label_list_is_refused() {
    for (text, message) in [
        (
            r#"{"tickets":[{"body":"B","labels":[]}]}"#,
            "tickets[0].title must be a non-empty string",
        ),
        (
            r#"{"tickets":[{"title":"","body":"B","labels":[]}]}"#,
            "tickets[0].title must be a non-empty string",
        ),
        (
            r#"{"tickets":[{"title":1,"body":"B","labels":[]}]}"#,
            "tickets[0].title must be a non-empty string",
        ),
        (
            r#"{"tickets":[{"title":"T","labels":[]}]}"#,
            "tickets[0].body must be a non-empty string",
        ),
        (
            r#"{"tickets":[{"title":"T","body":"  ","labels":[]}]}"#,
            "tickets[0].body must be a non-empty string",
        ),
        (
            r#"{"tickets":[{"title":"T","body":"B"}]}"#,
            "tickets[0].labels must be a list of names",
        ),
        (
            r#"{"tickets":[{"title":"T","body":"B","labels":"x"}]}"#,
            "tickets[0].labels must be a list of names",
        ),
        (
            r#"{"tickets":[{"title":"T","body":"B","labels":[1]}]}"#,
            "tickets[0].labels must be a list of names",
        ),
    ] {
        assert_eq!(tickets(text), Err(message.to_string()), "{text}");
    }
}

#[test]
fn a_ticket_that_is_not_a_mapping_is_refused() {
    assert_eq!(
        tickets(r#"{"tickets":["First"]}"#),
        Err("tickets[0].title must be a non-empty string".to_string())
    );
}

#[test]
fn no_arguments_are_usage() {
    let mut tracker = Fake::new(Vec::new());
    assert_eq!(run_with(&mut tracker, Vec::new()), 2);
    assert!(tracker.calls.is_empty());
}

#[test]
fn an_unknown_subcommand_is_usage() {
    let mut tracker = Fake::new(Vec::new());
    let argv = vec![OsString::from("send"), os(&fixture("repo"))];
    assert_eq!(run_with(&mut tracker, argv), 2);
    assert!(tracker.calls.is_empty());
}

#[test]
fn the_wrong_number_of_arguments_is_usage() {
    let repo = Temp::new("usage").setup(SETUP).tickets(TICKETS);
    for argv in [
        vec![OsString::from("check")],
        vec![
            OsString::from("check"),
            os(repo.path()),
            OsString::from("extra"),
        ],
        vec![OsString::from("publish"), os(repo.path())],
        vec![
            OsString::from("publish"),
            os(repo.path()),
            os(&repo.tickets_path()),
            OsString::from("extra"),
        ],
    ] {
        let mut tracker = Fake::ok(2);
        assert_eq!(run_with(&mut tracker, argv), 2);
        assert!(tracker.calls.is_empty());
    }
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
        let dir = std::env::temp_dir().join(format!("ticket-publish-script-{pid}-{name}"));
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
fn gh_cli_sends_the_issue_create_arguments_and_the_body_on_stdin() {
    let script = Script::new("args", "#!/bin/sh\nprintf '%s ' \"$@\"\ncat\n");
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.create(
            REPO,
            "First",
            "Body one.\n",
            &["needs-triage".to_string(), "type: feature".to_string()],
        ),
        Ok("issue create --repo acme/kit --title First --body-file - --label needs-triage --label type: feature Body one.".to_string())
    );
}

#[test]
fn gh_cli_returns_the_last_non_empty_line() {
    let script = Script::new(
        "url",
        "#!/bin/sh\ncat > /dev/null\necho 'warning: something'\necho 'https://github.com/acme/kit/issues/7'\necho\necho\n",
    );
    let mut gh = script.client(Duration::from_secs(10));
    assert_eq!(
        gh.create(REPO, "T", "B", &[]),
        Ok("https://github.com/acme/kit/issues/7".to_string())
    );
}

#[test]
fn gh_cli_reports_a_failure_with_the_stderr_text() {
    let script = Script::new(
        "boom",
        "#!/bin/sh\ncat > /dev/null\necho 'boom' >&2\nexit 1\n",
    );
    let mut gh = script.client(Duration::from_secs(10));
    match gh.create(REPO, "T", "B", &[]) {
        Err(message) => {
            assert!(message.contains("boom"), "{message}");
            assert!(message.contains("acme/kit"), "{message}");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn gh_cli_refuses_an_answer_without_a_url() {
    let script = Script::new("empty", "#!/bin/sh\ncat > /dev/null\nexit 0\n");
    let mut gh = script.client(Duration::from_secs(10));
    match gh.create(REPO, "T", "B", &[]) {
        Err(message) => assert!(message.contains("no issue URL"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn gh_cli_kills_a_hung_call() {
    let script = Script::new("hung", "#!/bin/sh\nexec sleep 5\n");
    let mut gh = script.client(Duration::from_millis(100));
    match gh.create(REPO, "T", "B", &[]) {
        Err(message) => assert!(message.contains("timed out after"), "{message}"),
        other => panic!("expected a timeout, got {other:?}"),
    }
}

#[test]
fn gh_cli_reports_a_missing_program() {
    let script = Script::new("missing", "#!/bin/sh\n");
    let missing = script.dir.join("no-such-gh");
    let mut gh = GhCli::with(missing.into_os_string(), Duration::from_secs(10));
    match gh.create(REPO, "T", "B", &[]) {
        Err(message) => assert!(message.contains("no-such-gh"), "{message}"),
        other => panic!("expected an unreadable program, got {other:?}"),
    }
}

#[test]
fn default_is_the_real_gh_with_a_minute() {
    assert_eq!(
        format!("{:?}", GhCli::default()),
        format!("{:?}", GhCli::new())
    );
}
