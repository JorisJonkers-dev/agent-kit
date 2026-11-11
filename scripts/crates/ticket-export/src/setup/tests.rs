//! Tests at the public seam: `tracker_repo` over a repository directory.
//!
//! The valid and the missing cases use the recorded fixtures; every invalid
//! case writes its own setup file into a per-test temporary repository.

use super::tracker_repo;
use std::path::{Path, PathBuf};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

/// A throwaway repository holding `docs/agents/`, removed when the test ends.
struct TempRepo {
    dir: PathBuf,
}

impl TempRepo {
    fn new(test: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ticket-export-setup-{}-{test}", std::process::id()));
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
    tracker_repo(repo.path()).expect_err("the setup file must be refused")
}

#[test]
fn reads_the_tracker_repo_from_the_recorded_fixture() {
    assert_eq!(tracker_repo(&fixture("repo")), Ok("acme/kit".to_string()));
}

#[test]
fn refuses_a_repository_without_a_setup_file() {
    let dir = fixture("repo-without-setup");
    assert_eq!(
        tracker_repo(&dir),
        Err(format!(
            "docs/agents/setup.yaml is missing in {}; run repo_setup.py write",
            dir.display()
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
    let repo = TempRepo::new("not-utf8").write_bytes(&[b'v', b'e', b'r', 0xff, 0xfe]);
    let err = refusal(&repo);
    assert!(
        err.starts_with("docs/agents/setup.yaml is not UTF-8: "),
        "{err}"
    );
}

#[test]
fn refuses_a_setup_file_that_is_not_yaml() {
    let repo = TempRepo::new("bad-yaml").write("version: [1\n");
    let err = refusal(&repo);
    assert!(
        err.starts_with("docs/agents/setup.yaml is not valid YAML: "),
        "{err}"
    );
}

#[test]
fn refuses_a_setup_file_that_is_not_a_mapping() {
    for (name, text) in [
        ("empty", ""),
        ("list", "- version: 1\n"),
        ("scalar", "version\n"),
    ] {
        let repo = TempRepo::new(&format!("not-a-mapping-{name}")).write(text);
        assert_eq!(
            refusal(&repo),
            "docs/agents/setup.yaml must be a mapping",
            "{name}"
        );
    }
}

#[test]
fn refuses_a_version_that_is_not_the_integer_one() {
    for (name, text) in [
        ("missing", "tracker: {kind: github, repo: acme/kit}\n"),
        (
            "two",
            "version: 2\ntracker: {kind: github, repo: acme/kit}\n",
        ),
        (
            "string",
            "version: '1'\ntracker: {kind: github, repo: acme/kit}\n",
        ),
        (
            "float",
            "version: 1.0\ntracker: {kind: github, repo: acme/kit}\n",
        ),
        (
            "bool",
            "version: true\ntracker: {kind: github, repo: acme/kit}\n",
        ),
    ] {
        let repo = TempRepo::new(&format!("version-{name}")).write(text);
        assert_eq!(
            refusal(&repo),
            "docs/agents/setup.yaml: version must be 1",
            "{name}"
        );
    }
}

#[test]
fn refuses_a_tracker_that_is_not_a_mapping() {
    for (name, text) in [
        ("missing", "version: 1\n"),
        ("list", "version: 1\ntracker: [github, acme/kit]\n"),
        ("scalar", "version: 1\ntracker: acme/kit\n"),
    ] {
        let repo = TempRepo::new(&format!("tracker-{name}")).write(text);
        assert_eq!(
            refusal(&repo),
            "docs/agents/setup.yaml: tracker must be a mapping with kind and repo",
            "{name}"
        );
    }
}

#[test]
fn refuses_a_tracker_kind_that_is_not_github() {
    for (name, text) in [
        ("missing", "version: 1\ntracker: {repo: acme/kit}\n"),
        (
            "gitlab",
            "version: 1\ntracker: {kind: gitlab, repo: acme/kit}\n",
        ),
        ("number", "version: 1\ntracker: {kind: 1, repo: acme/kit}\n"),
    ] {
        let repo = TempRepo::new(&format!("kind-{name}")).write(text);
        assert_eq!(
            refusal(&repo),
            "docs/agents/setup.yaml: tracker.kind must be github",
            "{name}"
        );
    }
}

#[test]
fn refuses_a_tracker_repo_that_is_not_owner_name() {
    for (name, text) in [
        ("missing", "version: 1\ntracker: {kind: github}\n"),
        (
            "no-slash",
            "version: 1\ntracker: {kind: github, repo: acme}\n",
        ),
        (
            "two-slashes",
            "version: 1\ntracker: {kind: github, repo: acme/kit/extra}\n",
        ),
        (
            "empty-owner",
            "version: 1\ntracker: {kind: github, repo: /kit}\n",
        ),
        (
            "empty-name",
            "version: 1\ntracker: {kind: github, repo: 'acme/'}\n",
        ),
        (
            "dot-owner",
            "version: 1\ntracker: {kind: github, repo: ./kit}\n",
        ),
        (
            "dotdot-owner",
            "version: 1\ntracker: {kind: github, repo: ../kit}\n",
        ),
        (
            "dots-name",
            "version: 1\ntracker: {kind: github, repo: acme/..}\n",
        ),
        (
            "space",
            "version: 1\ntracker: {kind: github, repo: 'acme/k it'}\n",
        ),
        (
            "not-a-string",
            "version: 1\ntracker: {kind: github, repo: 42}\n",
        ),
    ] {
        let repo = TempRepo::new(&format!("repo-{name}")).write(text);
        assert_eq!(
            refusal(&repo),
            "docs/agents/setup.yaml: tracker.repo must be OWNER/NAME",
            "{name}"
        );
    }
}

#[test]
fn accepts_owner_and_name_with_every_allowed_character() {
    let repo = TempRepo::new("allowed-characters")
        .write("version: 1\ntracker: {kind: github, repo: acme.io_1/kit.rs-tools_2}\n");
    assert_eq!(
        tracker_repo(repo.path()),
        Ok("acme.io_1/kit.rs-tools_2".to_string())
    );
}

#[test]
fn ignores_the_keys_it_does_not_use() {
    let repo = TempRepo::new("extra-keys").write(
        "version: 1\n\
         tracker: {kind: github, repo: acme/kit, extra: 1}\n\
         labels: [a, b]\n\
         triage_labels: {needs-triage: a}\n\
         flaky_jobs: []\n\
         unknown: {deep: [1, 2]}\n",
    );
    assert_eq!(tracker_repo(repo.path()), Ok("acme/kit".to_string()));
}
