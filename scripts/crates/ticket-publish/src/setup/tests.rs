//! Tests at the public seam: `labels` over a repository directory.
//!
//! The valid and the missing cases use the recorded fixtures; every invalid
//! case writes its own setup file into a per-test temporary repository.

use super::labels;
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
        let dir = std::env::temp_dir().join(format!(
            "ticket-publish-setup-{}-{test}",
            std::process::id()
        ));
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
    labels(repo.path()).expect_err("the setup file must be refused")
}

#[test]
fn reads_the_label_vocabulary_from_the_recorded_fixture() {
    assert_eq!(
        labels(&fixture("repo")),
        Ok(vec![
            "type: feature".to_string(),
            "needs-triage".to_string(),
            "needs-info".to_string(),
            "ready-for-agent".to_string(),
            "ready-for-human".to_string(),
            "wontfix".to_string(),
        ])
    );
}

#[test]
fn refuses_a_repository_without_a_setup_file() {
    let dir = fixture("repo-without-setup");
    assert_eq!(
        labels(&dir),
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
    let repo = TempRepo::new("not-utf8").write_bytes(&[b'l', b'a', b'b', 0xff, 0xfe]);
    let err = refusal(&repo);
    assert!(
        err.starts_with("docs/agents/setup.yaml is not UTF-8: "),
        "{err}"
    );
}

#[test]
fn refuses_a_setup_file_that_is_not_yaml() {
    let repo = TempRepo::new("bad-yaml").write("labels: [a\n");
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
        ("list", "- labels: [a]\n"),
        ("scalar", "labels\n"),
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
fn refuses_labels_that_are_not_a_non_empty_list_of_names() {
    for (name, text) in [
        ("missing", "version: 1\n"),
        ("empty", "labels: []\n"),
        ("scalar", "labels: needs-triage\n"),
        ("mapping", "labels: {a: b}\n"),
        ("number", "labels: [1]\n"),
        ("empty-name", "labels: ['']\n"),
        ("blank-name", "labels: ['  ']\n"),
        ("null-item", "labels: [~]\n"),
    ] {
        let repo = TempRepo::new(&format!("labels-{name}")).write(text);
        assert_eq!(
            refusal(&repo),
            "docs/agents/setup.yaml: labels must be a non-empty list of names",
            "{name}"
        );
    }
}

#[test]
fn ignores_the_keys_it_does_not_use() {
    let repo = TempRepo::new("extra-keys").write(
        "version: 1\n\
         tracker: {kind: github, repo: acme/kit}\n\
         labels: [a, b]\n\
         triage_labels: {needs-triage: a}\n\
         flaky_jobs: []\n\
         unknown: {deep: [1, 2]}\n",
    );
    assert_eq!(
        labels(repo.path()),
        Ok(vec!["a".to_string(), "b".to_string()])
    );
}
