//! The skill sources drift check, driven against a fake GitHub CLI.

use super::support::{run_with, Run, Tmp};
use crate::skill_sources::{
    civil, current_blob, is_transient, latest_tag, parse, run, run_with as drive, skills,
    sync_issue, today, update, validate, Gh, Subprocess,
};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

const ISSUE_TITLE: &str = "Skill sources drift";
const BLOB_OLD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BLOB_NEW: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FILE_KEY: &str = "repos/JorisJonkers-dev/deploy-kit/contents/spec/v1/10-project-intent.md";
const RELEASE_KEY: &str = "repos/JorisJonkers-dev/github-workflows/releases/latest";

fn file_sources() -> String {
    format!("sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/10-project-intent.md\n    blob: {BLOB_OLD}\n    checked: 2026-11-03\n")
}

const RELEASE_SOURCES: &str = "sources:\n  - kind: release\n    repo: JorisJonkers-dev/github-workflows\n    tag: v0.43.0\n    checked: 2026-11-03\n";

fn both_sources() -> String {
    format!(
        "{}{}",
        file_sources(),
        &RELEASE_SOURCES["sources:\n".len()..]
    )
}

fn moved_line() -> String {
    format!("estate-bootstrap: JorisJonkers-dev/deploy-kit spec/v1/10-project-intent.md moved (blob {BLOB_OLD} -> {BLOB_NEW})")
}

/// A fake `gh`: `api` serves JSON by path, `issue` keeps its issues, every call is logged.
#[derive(Default)]
struct Fake {
    api: HashMap<String, Result<String, String>>,
    issues: Vec<(u64, String, String, bool)>,
    calls: Vec<String>,
}

impl Fake {
    fn serving(entries: &[(&str, &str)]) -> Fake {
        Fake {
            api: entries
                .iter()
                .map(|(key, body)| (key.to_string(), Ok(body.to_string())))
                .collect(),
            ..Fake::default()
        }
    }

    fn failing(mut self, key: &str, error: &str) -> Fake {
        self.api.insert(key.to_string(), Err(error.to_string()));
        self
    }

    fn seeded(mut self, body: &str) -> Fake {
        self.issues
            .push((7, ISSUE_TITLE.to_string(), body.to_string(), true));
        self
    }

    fn called(&self, needle: &str) -> bool {
        self.calls.iter().any(|call| call.contains(needle))
    }
}

impl Gh for Fake {
    fn gh(&mut self, args: &[&str]) -> Result<String, String> {
        self.calls.push(args.join(" "));
        match args {
            ["api", rest @ ..] => {
                let path = rest.iter().find(|arg| arg.starts_with("repos/")).unwrap();
                self.api
                    .get(*path)
                    .cloned()
                    .unwrap_or(Err("gh: Not Found (HTTP 404)".to_string()))
            }
            ["issue", "list", ..] => {
                let open: Vec<String> = self
                    .issues
                    .iter()
                    .filter(|issue| issue.3)
                    .map(|issue| format!("{{\"number\": {}, \"title\": {:?}}}", issue.0, issue.1))
                    .collect();
                Ok(format!("[{}]", open.join(",")))
            }
            ["issue", "create", "--title", title, "--body", body] => {
                let number = self.issues.iter().map(|issue| issue.0).max().unwrap_or(0) + 1;
                self.issues
                    .push((number, title.to_string(), body.to_string(), true));
                Ok(String::new())
            }
            ["issue", "edit", number, "--body", body] => {
                self.issues
                    .iter_mut()
                    .filter(|issue| issue.0.to_string() == *number)
                    .for_each(|issue| issue.2 = body.to_string());
                Ok(String::new())
            }
            ["issue", "close", number] => {
                self.issues
                    .iter_mut()
                    .filter(|issue| issue.0.to_string() == *number)
                    .for_each(|issue| issue.3 = false);
                Ok(String::new())
            }
            other => Err(format!("fake gh: unsupported invocation: {other:?}")),
        }
    }
}

fn skill(root: &Path, name: &str, sources: Option<&str>) {
    fs::create_dir_all(root.join(name)).unwrap();
    if let Some(body) = sources {
        fs::write(root.join(name).join("sources.yaml"), body).unwrap();
    }
}

fn check(root: &Path, fake: &mut Fake, extra: &[&str]) -> Run {
    let root = root.display().to_string();
    let mut args: Vec<String> = vec!["--root".into(), root];
    args.extend(extra.iter().map(|arg| arg.to_string()));
    run_with(|io| drive(&args, io, fake, "2026-11-15"), "")
}

fn workstation(tmp: &Tmp, sources: &str) -> std::path::PathBuf {
    let root = tmp.join("workstation");
    skill(&root, "estate-bootstrap", Some(sources));
    root
}

#[test]
fn an_unchanged_source_reports_no_drift() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &both_sources());
    let mut fake = Fake::serving(&[
        (FILE_KEY, &format!("{{\"sha\": \"{BLOB_OLD}\"}}")),
        (RELEASE_KEY, "{\"tag_name\": \"v0.43.0\"}"),
    ]);
    let result = check(&root, &mut fake, &[]);
    assert_eq!(result.code, 0, "{}", result.out);
    assert!(result.out.contains("no sources moved"));
}

#[test]
fn a_moved_blob_is_listed() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    let result = check(
        &root,
        &mut Fake::serving(&[(FILE_KEY, &format!("{{\"sha\": \"{BLOB_NEW}\"}}"))]),
        &[],
    );
    assert_eq!(result.code, 1);
    assert!(result.out.contains(&moved_line()));
    assert!(result.out.contains("1 source(s) moved"));
}

#[test]
fn a_missing_path_is_listed() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &both_sources());
    let mut fake = Fake::serving(&[
        (RELEASE_KEY, "{\"tag_name\": \"v0.43.0\"}"),
        ("repos/JorisJonkers-dev/deploy-kit", "{}"),
    ]);
    let result = check(&root, &mut fake, &[]);
    assert_eq!(result.code, 1);
    assert!(result.out.contains(
        "estate-bootstrap: JorisJonkers-dev/deploy-kit spec/v1/10-project-intent.md is gone"
    ));
}

#[test]
fn a_404_on_an_invisible_repo_is_a_failure_not_a_gone_source() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    let result = check(&root, &mut Fake::default(), &[]);
    assert_eq!(result.code, 1);
    assert!(result
        .out
        .contains("not found or not visible to this token"));
    assert!(!result.out.contains("is gone"));
    assert!(result.out.contains("1 check(s) failed"));
}

#[test]
fn a_repo_check_that_fails_otherwise_is_a_failure() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    let mut fake = Fake::default().failing("repos/JorisJonkers-dev/deploy-kit", "gh: HTTP 500");
    let result = check(&root, &mut fake, &[]);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("check failed: gh: HTTP 500"));
}

#[test]
fn a_gone_release_and_a_newer_release_are_listed() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, RELEASE_SOURCES);
    let mut fake = Fake::serving(&[(RELEASE_KEY, "{\"tag_name\": \"v0.44.0\"}")]);
    let result = check(&root, &mut fake, &[]);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("estate-bootstrap: JorisJonkers-dev/github-workflows has a newer release v0.44.0 (recorded v0.43.0)"));
    let mut gone = Fake::serving(&[("repos/JorisJonkers-dev/github-workflows", "{}")]);
    let result = check(&root, &mut gone, &[]);
    assert!(result.out.contains("release is gone (recorded v0.43.0)"));
    let mut broken = Fake::default().failing(RELEASE_KEY, "gh: HTTP 401: Bad credentials");
    assert!(check(&root, &mut broken, &[])
        .out
        .contains("release check failed: gh: HTTP 401"));
    let mut unseen = Fake::default();
    assert!(check(&root, &mut unseen, &[])
        .out
        .contains("release check failed: JorisJonkers-dev/github-workflows: not found"));
}

#[test]
fn a_failed_request_is_a_failure_and_never_drift() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &both_sources());
    let mut fake = Fake::serving(&[(RELEASE_KEY, "{\"tag_name\": \"v0.43.0\"}")])
        .failing(FILE_KEY, "gh: HTTP 401: Bad credentials");
    let result = check(&root, &mut fake, &[]);
    assert_eq!(result.code, 1);
    assert!(result
        .out
        .contains("check failed: gh: HTTP 401: Bad credentials"));
    assert!(result.out.contains("1 check(s) failed"));
    for word in ["moved", "gone", "newer"] {
        assert!(!result.out.contains(word), "{word}");
    }
}

#[test]
fn replies_that_are_not_what_github_sends_fail() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &both_sources());
    for (file, release, wanted) in [
        ("not json", "{}", "GitHub's reply is not JSON"),
        ("{}", "{}", "names no blob"),
        ("{\"sha\": 5}", "{}", "names no blob"),
        (
            &format!("{{\"sha\": \"{BLOB_OLD}\"}}"),
            "{}",
            "names no release tag",
        ),
        (
            &format!("{{\"sha\": \"{BLOB_OLD}\"}}"),
            "nope",
            "GitHub's reply is not JSON",
        ),
    ] {
        let result = check(
            &root,
            &mut Fake::serving(&[(FILE_KEY, file), (RELEASE_KEY, release)]),
            &[],
        );
        assert_eq!(result.code, 1);
        assert!(result.out.contains(wanted), "{}", result.out);
    }
}

#[test]
fn a_missing_root_fails_and_leaves_the_issue_alone() {
    let tmp = Tmp::new();
    let mut fake = Fake::default().seeded("old");
    let result = check(&tmp.join("nowhere"), &mut fake, &["--sync-issue"]);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("no such skills folder"));
    assert!(fake.issues[0].3);
}

#[test]
fn the_issue_lookup_asks_for_more_than_the_default_page() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    let mut fake = Fake::serving(&[(FILE_KEY, &format!("{{\"sha\": \"{BLOB_OLD}\"}}"))]);
    check(&root, &mut fake, &["--sync-issue"]);
    assert!(fake.called("issue list --state open --limit 1000"));
}

#[test]
fn validate_modes() {
    let tmp = Tmp::new();
    let root = tmp.join("workstation");
    fs::create_dir_all(root.join("pr-composer")).unwrap();
    fs::write(root.join("estate-notes"), "a file, not a skill").unwrap();
    let result = check(&root, &mut Fake::default(), &["--validate"]);
    assert_eq!(
        (result.code, result.out.as_str()),
        (0, "every sources.yaml is well formed\n")
    );

    skill(&root, "estate-bootstrap", None);
    let result = check(&root, &mut Fake::default(), &["--validate"]);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("estate-bootstrap: has no sources.yaml"));
    assert!(result.out.contains("1 problem(s)"));

    for (body, wanted) in [
        ("sources: []\n", "lists no sources"),
        ("sources: text\n", "does not hold a sources list"),
        ("- not: a mapping\n", "does not hold a sources list"),
        ("sources: [unclosed\n", "cannot be read"),
    ] {
        skill(&root, "estate-bootstrap", Some(body));
        let result = check(&root, &mut Fake::default(), &["--validate"]);
        assert_eq!(result.code, 1, "{body}");
        assert!(result.out.contains(wanted), "{}", result.out);
    }
    fs::write(root.join("estate-bootstrap/sources.yaml"), [0xff, 0xfe]).unwrap();
    assert!(check(&root, &mut Fake::default(), &["--validate"])
        .out
        .contains("cannot be read"));
}

#[test]
fn validate_fails_on_malformed_entries() {
    let tmp = Tmp::new();
    let root = tmp.join("workstation");
    let bad = [
        "sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/a.md\n    blob: not-a-sha\n    checked: 2026-11-03\n",
        "sources:\n  - kind: branch\n    repo: JorisJonkers-dev/deploy-kit\n    checked: 2026-11-03\n",
        &format!("sources:\n  - kind: file\n    repo: deploy-kit\n    path: spec/v1/a.md\n    blob: {BLOB_OLD}\n    checked: 2026-11-03\n"),
        &format!("sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/a.md\n    blob: {BLOB_OLD}\n    checked: 03/11/2026\n"),
        &format!("sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/a.md\n    blob: {BLOB_OLD}\n    checked: 2026-02-30\n"),
        &format!("sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/a.md\n    blob: {BLOB_OLD}\n    checked: 2026-13-01\n"),
        &format!("sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/a.md\n    blob: {BLOB_OLD}\n    checked: 2026-1-01\n"),
        &format!("sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/a.md\n    blob: {BLOB_OLD}\n    checked: 2026-11\n"),
        &format!("sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: ''\n    blob: {BLOB_OLD}\n    checked: 2026-11-03\n"),
        "sources:\n  - kind: release\n    repo: JorisJonkers-dev/github-workflows\n    checked: 2026-11-03\n",
        "sources:\n  - kind: release\n    repo: -bad/name\n    tag: v1\n    checked: 2026-11-03\n",
        "sources:\n  - kind: release\n    repo: owner/\n    tag: v1\n    checked: 2026-11-03\n",
        "sources:\n  - just text\n",
    ];
    for body in bad {
        skill(&root, "estate-bootstrap", Some(body));
        let result = check(&root, &mut Fake::default(), &["--validate"]);
        assert_eq!(result.code, 1, "{body}");
        assert!(
            result
                .out
                .contains("estate-bootstrap: sources.yaml entry 1"),
            "{body}"
        );
        assert!(!result.out.contains("well formed"), "{body}");
    }
    // A leap day is a date; a date YAML itself reads as a date is accepted too.
    let good = format!("sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/a.md\n    blob: {BLOB_OLD}\n    checked: 2024-02-29\n");
    skill(&root, "estate-bootstrap", Some(&good));
    assert_eq!(check(&root, &mut Fake::default(), &["--validate"]).code, 0);
    skill(&root, "estate-bootstrap", Some(&both_sources()));
    assert_eq!(check(&root, &mut Fake::default(), &["--validate"]).code, 0);
    assert_eq!(validate(&root), Vec::<String>::new());
    assert_eq!(skills(&root).len(), 1);
    assert_eq!(parse(&root.join("estate-bootstrap")).0.len(), 2);
}

#[test]
fn a_drift_check_stops_on_a_malformed_file() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, "sources: []\n");
    let result = check(&root, &mut Fake::default(), &[]);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("lists no sources"));
}

#[test]
fn update_rewrites_blobs_tags_and_dates() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &both_sources());
    let mut fake = Fake::serving(&[
        (FILE_KEY, &format!("{{\"sha\": \"{BLOB_NEW}\"}}")),
        (RELEASE_KEY, "{\"tag_name\": \"v0.44.0\"}"),
    ]);
    let result = check(&root, &mut fake, &["--update", "estate-bootstrap"]);
    assert_eq!(result.code, 0, "{}", result.out);
    assert!(result
        .out
        .starts_with("estate-bootstrap: recording what GitHub holds today (2026-11-15)"));
    let written = fs::read_to_string(root.join("estate-bootstrap/sources.yaml")).unwrap();
    assert!(written.starts_with("sources:\n- kind: file\n  repo: JorisJonkers-dev/deploy-kit\n  path: spec/v1/10-project-intent.md\n"), "{written}");
    let entries = parse(&root.join("estate-bootstrap")).0;
    assert_eq!(
        (entries[0].blob.as_str(), entries[1].tag.as_str()),
        (BLOB_NEW, "v0.44.0")
    );
    assert!(written.contains("2026-11-15"));
}

#[test]
fn update_leaves_the_file_alone_when_it_cannot_finish() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    let mut fake = Fake::default().failing(FILE_KEY, "gh: HTTP 403: API rate limit exceeded");
    let result = check(&root, &mut fake, &["--update", "estate-bootstrap"]);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("rate limit"));
    assert_eq!(
        fs::read_to_string(root.join("estate-bootstrap/sources.yaml")).unwrap(),
        file_sources()
    );

    let mut gone = Fake::serving(&[("repos/JorisJonkers-dev/deploy-kit", "{}")]);
    assert!(check(&root, &mut gone, &["--update", "estate-bootstrap"])
        .out
        .contains("is gone; nothing to record"));
    let releases = workstation(&tmp, RELEASE_SOURCES);
    let mut none = Fake::serving(&[("repos/JorisJonkers-dev/github-workflows", "{}")]);
    assert!(
        check(&releases, &mut none, &["--update", "estate-bootstrap"])
            .out
            .contains("has no release; nothing to record")
    );
}

#[test]
fn update_refuses_names_and_files_it_cannot_use() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    for name in ["pr-composer", "estate-a/b", "../estate-x"] {
        let result = check(&root, &mut Fake::default(), &["--update", name]);
        assert!(
            result.out.contains("not an estate skill folder name"),
            "{name}"
        );
    }
    assert!(
        check(&root, &mut Fake::default(), &["--update", "estate-none"])
            .out
            .contains("no such skill under")
    );
    skill(&root, "estate-bad", Some("sources: []\n"));
    assert!(
        check(&root, &mut Fake::default(), &["--update", "estate-bad"])
            .out
            .contains("lists no sources")
    );
}

#[test]
fn update_reports_a_file_it_cannot_write() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    // The file is swapped for a directory while the lookup runs, so the write fails.
    struct Swap(std::path::PathBuf);
    impl Gh for Swap {
        fn gh(&mut self, _: &[&str]) -> Result<String, String> {
            fs::remove_file(&self.0).unwrap();
            fs::create_dir(&self.0).unwrap();
            Ok(format!("{{\"sha\": \"{BLOB_NEW}\"}}"))
        }
    }
    let result = update(
        &root,
        "estate-bootstrap",
        "2026-11-15",
        &mut Swap(root.join("estate-bootstrap/sources.yaml")),
    );
    assert!(result.unwrap_err().contains("directory"));
}

#[test]
fn sync_issue_keeps_one_issue_in_step() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    let old = format!("{{\"sha\": \"{BLOB_OLD}\"}}");
    let new = format!("{{\"sha\": \"{BLOB_NEW}\"}}");

    let mut fake = Fake::serving(&[(FILE_KEY, &old)]);
    let result = check(&root, &mut fake, &["--sync-issue"]);
    assert_eq!(result.code, 0);
    assert!(fake.issues.is_empty() && !fake.called("issue create"));
    assert!(result
        .out
        .contains("\"Skill sources drift\": no drift and no issue open"));

    let mut fake = Fake::serving(&[(FILE_KEY, &new)]);
    let result = check(&root, &mut fake, &["--sync-issue"]);
    assert_eq!(result.code, 0);
    assert_eq!(
        (
            fake.issues.len(),
            fake.issues[0].1.as_str(),
            fake.issues[0].3
        ),
        (1, ISSUE_TITLE, true)
    );
    assert!(fake.issues[0].2.contains(&moved_line()));
    assert!(fake.issues[0]
        .2
        .contains("ci-helpers skill-sources --update <skill>"));
    assert!(result.out.contains("opened"));

    let mut fake = Fake::serving(&[(FILE_KEY, &new)]).seeded("old body");
    let result = check(
        &root,
        &mut fake,
        &["--sync-issue", "--issue-title", ISSUE_TITLE],
    );
    assert_eq!(result.code, 0);
    assert_eq!((fake.issues.len(), fake.issues[0].0), (1, 7));
    assert!(!fake.issues[0].2.contains("old body") && fake.issues[0].2.contains(&moved_line()));
    assert!(fake.called("issue edit 7") && !fake.called("issue create"));
    assert!(result.out.contains("updated"));

    let mut fake = Fake::serving(&[(FILE_KEY, &old)]).seeded("old body");
    let result = check(&root, &mut fake, &["--sync-issue"]);
    assert_eq!(result.code, 0);
    assert!(!fake.issues[0].3 && fake.called("issue close 7"));
    assert!(result.out.contains("closed"));
}

#[test]
fn a_failed_run_leaves_the_issue_untouched() {
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    let mut fake = Fake::default()
        .failing(FILE_KEY, "gh: HTTP 401: Bad credentials")
        .seeded("old body");
    let result = check(&root, &mut fake, &["--sync-issue"]);
    assert_eq!(result.code, 1);
    assert!(result
        .out
        .contains("check failed: gh: HTTP 401: Bad credentials"));
    assert!(result.out.contains("issue is left untouched"));
    assert_eq!(
        fake.issues,
        vec![(7, ISSUE_TITLE.to_string(), "old body".to_string(), true)]
    );
    for verb in ["issue create", "issue edit", "issue close"] {
        assert!(!fake.called(verb));
    }
}

#[test]
fn an_issue_failure_is_reported() {
    let tmp = Tmp::new();
    let mut fake = Fake::default();
    assert!(sync_issue(&mut fake, ISSUE_TITLE, &[]).is_ok());
    struct Broken(&'static str);
    impl Gh for Broken {
        fn gh(&mut self, _: &[&str]) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }
    assert!(sync_issue(&mut Broken("nope"), ISSUE_TITLE, &[])
        .unwrap_err()
        .contains("not JSON"));
    assert_eq!(
        sync_issue(&mut Broken("{}"), ISSUE_TITLE, &[]),
        Ok("\"Skill sources drift\": no drift and no issue open".to_string())
    );
    assert_eq!(
        sync_issue(
            &mut Broken("[1, {\"title\": \"Skill sources drift\"}]"),
            ISSUE_TITLE,
            &[]
        ),
        Ok("\"Skill sources drift\": no drift and no issue open".to_string())
    );
    let root = workstation(&tmp, &file_sources());
    let mut fake = Fake::serving(&[(FILE_KEY, &format!("{{\"sha\": \"{BLOB_NEW}\"}}"))]);
    fake.issues.clear();
    struct NoCreate(Fake);
    impl Gh for NoCreate {
        fn gh(&mut self, args: &[&str]) -> Result<String, String> {
            if args[..2] == ["issue", "create"] {
                Err("boom".to_string())
            } else {
                self.0.gh(args)
            }
        }
    }
    let result = run_with(
        |io| {
            drive(
                &[
                    "--root".into(),
                    root.display().to_string(),
                    "--sync-issue".into(),
                ],
                io,
                &mut NoCreate(fake),
                "2026-11-15",
            )
        },
        "",
    );
    assert_eq!(result.code, 1);
    assert!(result.out.contains("check failed: boom"));
}

#[test]
fn command_line_errors() {
    for args in [
        vec!["--bogus"],
        vec!["--root"],
        vec!["--update"],
        vec!["--issue-title"],
        vec!["--validate", "--sync-issue"],
        vec!["--update=estate-x", "--validate"],
    ] {
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        let result = run_with(
            |io| drive(&args, io, &mut Fake::default(), "2026-11-15"),
            "",
        );
        assert_eq!(result.code, 2, "{args:?}");
        assert!(result.err.starts_with("skill-sources: "));
    }
    let tmp = Tmp::new();
    let root = workstation(&tmp, &file_sources());
    let args = vec![
        format!("--root={}", root.display()),
        "--validate".to_string(),
    ];
    assert_eq!(
        run_with(|io| drive(&args, io, &mut Fake::default(), "x"), "").code,
        0
    );
    // The real entry point reads the real clock and the default root, which is absent here.
    let result = run_with(|io| run(&["--validate".to_string()], io), "");
    assert_eq!(result.code, 1);
    assert!(result.out.contains("no such skills folder"));
}

#[test]
fn blob_and_tag_lookups_encode_the_path() {
    let mut fake = Fake::serving(&[
        ("repos/o/r/contents/docs/a%20b.md", "{\"sha\": \"x\"}"),
        ("repos/o/r/releases/latest", "{\"tag_name\": \"v1\"}"),
    ]);
    assert_eq!(
        current_blob(&mut fake, "o", "docs/a b.md").unwrap_err(),
        "o: not found or not visible to this token (HTTP 404)"
    );
    let mut fake = Fake::serving(&[
        ("repos/o/r/contents/docs/a%20b.md", "{\"sha\": \"x\"}"),
        ("repos/o/r/releases/latest", "{\"tag_name\": \"v1\"}"),
    ]);
    assert_eq!(
        current_blob(&mut fake, "o/r", "docs/a b.md"),
        Ok(Some("x".to_string()))
    );
    assert_eq!(latest_tag(&mut fake, "o/r"), Ok(Some("v1".to_string())));
}

#[test]
fn civil_dates() {
    assert_eq!(civil(0), "1970-01-01");
    assert_eq!(civil(19_782), "2024-02-29");
    assert_eq!(civil(20_407), "2025-11-15");
    assert_eq!(civil(-1), "1969-12-31");
    let now = today();
    assert_eq!(
        (now.len(), now.as_bytes()[4], now.as_bytes()[7]),
        (10, b'-', b'-')
    );
}

#[test]
fn transient_failures() {
    for word in [
        "gh: timeout awaiting response",
        "Timed Out",
        "deadline exceeded",
        "connection reset",
        "HTTP 503",
    ] {
        assert!(is_transient(word), "{word}");
    }
    assert!(!is_transient("gh: HTTP 401: Bad credentials"));
}

/// A stand-in `gh` shell script that runs `body`.
#[path = "../../../../test-support/fake_exe.rs"]
mod fake_exe;

fn stub(tmp: &Tmp, body: &str) -> String {
    let path = tmp.join("gh");
    fake_exe::write_script(&path, &format!("#!/bin/sh\n{body}\n"));
    path.display().to_string()
}

fn subprocess(program: String, millis: u64) -> Subprocess {
    Subprocess {
        program,
        timeout: Duration::from_millis(millis),
        retries: 3,
    }
}

#[test]
fn the_subprocess_runner_returns_output() {
    let tmp = Tmp::new();
    let mut gh = subprocess(stub(&tmp, "echo \"$@\""), 5000);
    assert_eq!(gh.gh(&["api", "x"]), Ok("api x\n".to_string()));
}

#[test]
fn the_subprocess_runner_fails_at_once_on_a_plain_error() {
    let tmp = Tmp::new();
    let counter = tmp.join("count");
    let body = format!(
        "echo x >> {}; echo 'gh: HTTP 401: Bad credentials' >&2; exit 1",
        counter.display()
    );
    let mut gh = subprocess(stub(&tmp, &body), 5000);
    assert_eq!(
        gh.gh(&["api", "x"]),
        Err("gh: HTTP 401: Bad credentials".to_string())
    );
    assert_eq!(fs::read_to_string(counter).unwrap().lines().count(), 1);
}

#[test]
fn the_subprocess_runner_retries_a_transient_error_a_bounded_number_of_times() {
    let tmp = Tmp::new();
    let counter = tmp.join("count");
    let body = format!(
        "echo x >> {}; echo 'gh: timeout awaiting response' >&2; exit 1",
        counter.display()
    );
    let mut gh = subprocess(stub(&tmp, &body), 5000);
    assert_eq!(
        gh.gh(&["api", "x"]),
        Err("gh: timeout awaiting response".to_string())
    );
    assert_eq!(fs::read_to_string(counter).unwrap().lines().count(), 3);
}

#[test]
fn the_subprocess_runner_names_an_exit_code_when_stderr_is_empty() {
    let tmp = Tmp::new();
    let mut gh = subprocess(stub(&tmp, "exit 4"), 5000);
    assert_eq!(gh.gh(&["api", "x"]), Err("gh exited 4".to_string()));
}

#[test]
fn a_hung_call_times_out_and_is_retried() {
    let tmp = Tmp::new();
    let counter = tmp.join("count");
    let body = format!("echo x >> {}; sleep 30", counter.display());
    let mut gh = subprocess(stub(&tmp, &body), 150);
    assert_eq!(
        gh.gh(&["api", "x"]),
        Err("timeout waiting for gh".to_string())
    );
    assert_eq!(fs::read_to_string(counter).unwrap().lines().count(), 3);
}

#[test]
fn a_missing_program_is_not_retried() {
    let tmp = Tmp::new();
    let mut gh = subprocess(tmp.join("no-such-gh").display().to_string(), 1000);
    assert!(gh.gh(&["api", "x"]).unwrap_err().contains("No such file"));
    let mut none = Subprocess {
        retries: 0,
        ..subprocess(String::new(), 10)
    };
    assert_eq!(none.gh(&[]), Err("gh made no attempt".to_string()));
    assert_eq!(Subprocess::default().program, "gh");
}
