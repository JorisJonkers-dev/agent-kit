//! Tests at the public seam: `load` over a repository directory.
//!
//! The valid and the missing cases use the recorded fixtures; every invalid
//! case writes its own setup file into a per-test temporary repository.

use super::{load, FlakyJob};
use std::path::{Path, PathBuf};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// The setup file's head, which each invalid case edits.
const HEAD: &str = "version: 1\ntracker:\n  kind: github\n  repo: acme/kit\n";

/// A setup file the tool accepts.
const VALID: &str = "version: 1\ntracker:\n  kind: github\n  repo: acme/kit\nflaky_jobs: []\n";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

/// A throwaway repository holding `docs/agents/`, removed when the test ends.
struct TempRepo {
    dir: PathBuf,
}

impl TempRepo {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ci-wait-setup-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs/agents")).expect("create the temporary repository");
        Self { dir }
    }

    fn write(self, text: &str) -> Self {
        std::fs::write(self.dir.join("docs/agents/setup.yaml"), text).expect("write setup.yaml");
        self
    }

    fn write_bytes(self, bytes: &[u8]) -> Self {
        std::fs::write(self.dir.join("docs/agents/setup.yaml"), bytes).expect("write setup.yaml");
        self
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn refusal(repo: &TempRepo) -> String {
    load(repo.path()).expect_err("the setup file must be refused")
}

#[test]
fn reads_the_tracker_repo_and_the_flaky_jobs() {
    let setup = load(&fixture("repo")).expect("the recorded setup file must load");
    assert_eq!(setup.tracker_repo, "acme/kit");
    assert_eq!(
        setup.flaky_jobs,
        vec![FlakyJob {
            name: "flaky-e2e".to_string(),
            reason: "the runner pool is being drained".to_string(),
        }]
    );
}

#[test]
fn refuses_a_repository_without_a_setup_file() {
    let repo = TempRepo::new("missing");
    assert_eq!(
        load(repo.path()),
        Err(format!(
            "docs/agents/setup.yaml is missing in {}; run repo_setup.py write",
            repo.path().display()
        ))
    );
}

#[test]
fn refuses_a_setup_file_that_cannot_be_read() {
    let repo = TempRepo::new("unreadable");
    std::fs::create_dir(repo.path().join("docs/agents/setup.yaml"))
        .expect("create a directory named setup.yaml");
    let err = refusal(&repo);
    assert!(
        err.starts_with("docs/agents/setup.yaml is not readable: "),
        "{err}"
    );
}

#[test]
fn refuses_a_setup_file_that_is_not_utf8() {
    let repo = TempRepo::new("not-utf8").write_bytes(b"version: 1\n\xff\n");
    let err = refusal(&repo);
    assert!(
        err.starts_with("docs/agents/setup.yaml is not UTF-8: "),
        "{err}"
    );
}

#[test]
fn refuses_a_setup_file_that_is_not_yaml() {
    let repo = TempRepo::new("not-yaml").write("version: [1\n");
    let err = refusal(&repo);
    assert!(
        err.starts_with("docs/agents/setup.yaml is not valid YAML: "),
        "{err}"
    );
}

#[test]
fn refuses_a_setup_file_that_is_not_a_mapping() {
    let repo = TempRepo::new("not-mapping").write("- 1\n- 2\n");
    assert_eq!(
        refusal(&repo),
        "docs/agents/setup.yaml must be a mapping".to_string()
    );
}

#[test]
fn refuses_a_version_that_is_not_one() {
    for text in [
        "tracker:\n  kind: github\n  repo: acme/kit\n",
        "version: 2\n",
    ] {
        let repo = TempRepo::new("version").write(text);
        assert_eq!(
            refusal(&repo),
            "docs/agents/setup.yaml: version must be 1".to_string()
        );
    }
}

#[test]
fn refuses_a_tracker_that_is_not_a_mapping() {
    for text in ["version: 1\n", "version: 1\ntracker: 3\n"] {
        let repo = TempRepo::new("tracker").write(text);
        assert_eq!(
            refusal(&repo),
            "docs/agents/setup.yaml: tracker must be a mapping with kind and repo".to_string()
        );
    }
}

#[test]
fn refuses_a_tracker_kind_that_is_not_github() {
    let repo =
        TempRepo::new("kind").write("version: 1\ntracker:\n  kind: gitlab\n  repo: acme/kit\n");
    assert_eq!(
        refusal(&repo),
        "docs/agents/setup.yaml: tracker.kind must be github".to_string()
    );
}

#[test]
fn refuses_a_tracker_repo_that_is_not_owner_name() {
    for text in [
        "version: 1\ntracker:\n  kind: github\n",
        "version: 1\ntracker:\n  kind: github\n  repo: acme\n",
        "version: 1\ntracker:\n  kind: github\n  repo: ./x\n",
        "version: 1\ntracker:\n  kind: github\n  repo: acme/..\n",
    ] {
        let repo = TempRepo::new("repo").write(text);
        assert_eq!(
            refusal(&repo),
            "docs/agents/setup.yaml: tracker.repo must be OWNER/NAME".to_string()
        );
    }
}

#[test]
fn refuses_flaky_jobs_that_are_not_a_list() {
    let repo = TempRepo::new("flaky-not-list").write(&format!("{HEAD}flaky_jobs: 3\n"));
    assert_eq!(
        refusal(&repo),
        "docs/agents/setup.yaml: flaky_jobs must be a list".to_string()
    );
}

#[test]
fn refuses_a_flaky_job_that_is_not_a_mapping() {
    let repo = TempRepo::new("flaky-not-mapping").write(&format!("{HEAD}flaky_jobs:\n- 3\n"));
    assert_eq!(
        refusal(&repo),
        "docs/agents/setup.yaml: a flaky job must be a mapping with name, reason, reviewed_by and reviewed_on".to_string()
    );
}

#[test]
fn refuses_a_flaky_job_without_a_name_a_reason_or_a_review() {
    let cases = [
        (
            "reason: r\n  reviewed_by: octo\n  reviewed_on: 2026-01-01\n",
            "name",
        ),
        (
            "name: \"\"\n  reason: r\n  reviewed_by: octo\n  reviewed_on: 2026-01-01\n",
            "name",
        ),
        (
            "name: flaky\n  reviewed_by: octo\n  reviewed_on: 2026-01-01\n",
            "reason",
        ),
        (
            "name: flaky\n  reason: r\n  reviewed_on: 2026-01-01\n",
            "reviewed_by",
        ),
        (
            "name: flaky\n  reason: r\n  reviewed_by: octo\n",
            "reviewed_on",
        ),
        (
            "name: flaky\n  reason: r\n  reviewed_by: \"\"\n  reviewed_on: 2026-01-01\n",
            "reviewed_by",
        ),
    ];
    for (entry, field) in cases {
        let repo = TempRepo::new("flaky-field").write(&format!("{HEAD}flaky_jobs:\n- {entry}"));
        assert_eq!(
            refusal(&repo),
            format!("docs/agents/setup.yaml: a flaky job needs a non-empty {field}")
        );
    }
}

#[test]
fn a_setup_file_without_flaky_jobs_has_none() {
    let repo =
        TempRepo::new("no-flaky").write("version: 1\ntracker:\n  kind: github\n  repo: acme/kit\n");
    let setup = load(repo.path()).expect("the setup file must load");
    assert!(setup.flaky_jobs.is_empty());
}

#[test]
fn extra_keys_are_ignored() {
    let repo = TempRepo::new("extra").write(&format!(
        "{VALID}labels:\n- \"type: feature\"\ntriage_labels:\n  needs-triage: needs-triage\n"
    ));
    let setup = load(repo.path()).expect("the setup file must load");
    assert_eq!(setup.tracker_repo, "acme/kit");
    assert!(setup.flaky_jobs.is_empty());
}
