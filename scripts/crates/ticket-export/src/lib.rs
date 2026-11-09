//! Exports a GitHub ticket to a requirement.md file (issue #355).
//!
//! A workflow should be able to start from a ticket without any agent holding
//! GitHub write access, so this crate only reads: every request is a GET by
//! construction (the [`GitHub`] trait has just the one method), and
//! requirement.md is written only after every read has succeeded.
//!
//! Usage: `ticket-export <OWNER/REPO> <ISSUE_NUMBER> <OUT_FILE>` — exit 0 on
//! success (prints `ticket-export: wrote <OUT_FILE>`), 1 on a refusal (missing
//! issue, closed blocker, unreadable repository), 2 on bad usage, and on 1 or
//! 2 no file is written. std only, by design (ADR-0014, docs/RUST.md): each
//! jq filter answers plain text, so there is no JSON to parse.

use std::ffi::OsString;
use std::fs;
use std::process::Command;

pub const NAME: &str = env!("CARGO_PKG_NAME");

const TITLE_JQ: &str = ".title";
const STATE_JQ: &str = ".state";
const BODY_JQ: &str = r#".body // """#;
/// The jq filter of the list reads (sub-issues, blocked-by): one
/// `<number>\t<state>\t<title>` line per issue.
const LIST_JQ: &str = r#".[] | "\(.number)\t\(.state)\t\(.title)""#;

/// One read: a GET of a REST `path`, answered as plain text through a jq
/// filter. The trait has no other method, so a client can only read.
pub trait GitHub {
    fn get(&mut self, path: &str, jq: &str) -> Result<String, ReadError>;
}

/// Why one read failed: `NotFound` is a 404; `Unreadable` carries the stderr
/// of whatever refused the read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    NotFound,
    Unreadable(String),
}

/// The real GitHub client, read-only through `gh`, which already holds the
/// owner's read credentials.
///
/// This one impl is the only code the unit tests do not execute: they drive
/// the [`GitHub`] trait with a recording fake instead, so by the workspace's
/// 100 percent line-coverage standard (docs/RUST.md) the lines below are the
/// ones they cannot reach — hence kept to a few.
#[derive(Default)]
pub struct GhCli;

impl GitHub for GhCli {
    fn get(&mut self, path: &str, jq: &str) -> Result<String, ReadError> {
        let run = Command::new("gh")
            .args(["api", "-X", "GET", path, "--jq", jq])
            .output();
        match run {
            Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
                if stderr.contains("404") || stderr.contains("Not Found") {
                    Err(ReadError::NotFound)
                } else {
                    Err(ReadError::Unreadable(stderr))
                }
            }
            Err(e) => Err(ReadError::Unreadable(e.to_string())),
        }
    }
}

/// One read whose failure is a refusal: `what` names the read in the refusal
/// message. `gh` answers each read with a trailing newline, which is not part
/// of the answer.
fn read(gh: &mut dyn GitHub, path: &str, jq: &str, what: &str) -> Result<String, String> {
    gh.get(path, jq)
        .map(|text| text.trim_end_matches('\n').to_string())
        .map_err(|err| match err {
            ReadError::NotFound => format!("{what}: not found"),
            ReadError::Unreadable(e) => format!("{what}: unreadable: {e}"),
        })
}

/// One `<number>\t<state>\t<title>` line of a list read, split in three.
fn split_entry(line: &str) -> Option<(&str, &str, &str)> {
    let mut fields = line.splitn(3, '\t');
    let number = fields.next()?;
    let state = fields.next()?;
    let title = fields.next()?;
    Some((number, state, title))
}

/// One list read (sub-issues, blockers): the issues behind a GET of `path`,
/// one `(number, state, title)` tuple per issue.
fn entries(
    gh: &mut dyn GitHub,
    path: &str,
    what: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let text = read(gh, path, LIST_JQ, what)?;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (number, state, title) =
            split_entry(line).ok_or_else(|| format!("{what}: unreadable line {line:?}"))?;
        out.push((number.to_string(), state.to_string(), title.to_string()));
    }
    Ok(out)
}

/// The acceptance criteria: the lines of the body under the heading
/// `## Acceptance criteria`, up to the next `## ` heading, blank run-in and
/// run-out lines dropped.
fn acceptance_criteria(body: &str) -> Vec<String> {
    let mut lines: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == "## Acceptance criteria" {
            inside = true;
        } else if trimmed.starts_with("## ") {
            inside = false;
        } else if inside {
            lines.push(line);
        }
    }
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.iter().map(|l| l.to_string()).collect()
}

/// One section of requirement.md: its heading and its lines, or `None` when
/// the ticket has nothing for the section — the file still names it.
fn section(heading: &str, lines: &[String]) -> String {
    if lines.is_empty() {
        format!("## {heading}\n\nNone")
    } else {
        format!("## {heading}\n\n{}", lines.join("\n"))
    }
}

/// Reads the whole ticket and returns its requirement.md, or the refusal
/// message. The file holds the title, body, acceptance criteria, sub-issue
/// list and blocked-by list, and nothing else: no comments, no edit history.
pub fn export(gh: &mut dyn GitHub, repo: &str, number: u64) -> Result<String, String> {
    let issue = format!("repos/{repo}/issues/{number}");
    let what = format!("issue #{number} in {repo}");
    let title = read(gh, &issue, TITLE_JQ, &what)?;
    // The state read is part of the ticket's read order: any read failing is
    // a refusal. requirement.md records nothing else about the issue itself,
    // so the value is not used.
    read(gh, &issue, STATE_JQ, &what)?;
    let body = read(gh, &issue, BODY_JQ, &what)?;
    let sub_issues = entries(gh, &format!("{issue}/sub_issues"), &what)?;
    let blocked_by = entries(gh, &format!("{issue}/dependencies/blocked_by"), &what)?;
    for (n, state, _) in &blocked_by {
        if state == "closed" {
            return Err(format!("blocker #{n} of issue #{number} is closed"));
        }
    }
    let criteria = acceptance_criteria(&body);
    let sub_lines: Vec<String> = sub_issues
        .iter()
        .map(|(n, s, t)| format!("- #{n} {t} ({s})"))
        .collect();
    let blocked_lines: Vec<String> = blocked_by
        .iter()
        .map(|(n, s, t)| format!("- #{n} {t} ({s})"))
        .collect();
    Ok(format!(
        "# {title} (#{number})\n\n## Body\n\n{body}\n\n{}\n\n{}\n\n{}\n",
        section("Acceptance criteria", &criteria),
        section("Sub-issues", &sub_lines),
        section("Blocked by", &blocked_lines)
    ))
}

/// One `<OWNER/REPO>` argument: two non-empty names and nothing after the
/// single slash.
fn is_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let owner = parts.next().is_some_and(|owner| !owner.is_empty());
    let name = parts.next().is_some_and(|name| !name.is_empty());
    owner && name && parts.next().is_none()
}

/// One `<ISSUE_NUMBER>` argument: a positive integer.
fn positive(shown: &str) -> Option<u64> {
    shown.parse::<u64>().ok().filter(|number| *number > 0)
}

fn parse_args(args: &[OsString]) -> Result<(String, u64, OsString), String> {
    if args.len() != 3 {
        return Err(format!(
            "expected exactly 3 arguments, found {}",
            args.len()
        ));
    }
    let repo = args[0].to_string_lossy().into_owned();
    if !is_repo(&repo) {
        return Err(format!("{repo:?} is not OWNER/REPO"));
    }
    let shown = args[1].to_string_lossy();
    let Some(number) = positive(&shown) else {
        return Err(format!("{shown:?} is not a positive integer"));
    };
    Ok((repo, number, args[2].clone()))
}

/// One written OUT_FILE: the exit code it earns, 0 after the write and 1 when
/// the write itself failed.
fn write_out(out: &OsString, markdown: String) -> i32 {
    match fs::write(out, markdown) {
        Ok(()) => {
            println!("{NAME}: wrote {}", out.to_string_lossy());
            0
        }
        Err(e) => {
            eprintln!("{NAME}: cannot write {}: {e}", out.to_string_lossy());
            1
        }
    }
}

/// Answers one command line with an exit code: 0 on success, 1 on a refusal
/// (missing issue, closed blocker, unreadable repository), 2 on bad usage.
/// OUT_FILE is written only after every read succeeded, so on 1 and 2 no
/// file is written. Success prints to stdout, problems to stderr.
pub fn run_with(gh: &mut dyn GitHub, args: Vec<OsString>) -> i32 {
    match parse_args(&args) {
        Err(usage) => {
            eprintln!("{NAME}: {usage}");
            eprintln!("usage: {NAME} <OWNER/REPO> <ISSUE_NUMBER> <OUT_FILE>");
            2
        }
        Ok((repo, number, out)) => match export(gh, &repo, number) {
            Err(refusal) => {
                eprintln!("{NAME}: {refusal}");
                1
            }
            Ok(markdown) => write_out(&out, markdown),
        },
    }
}

/// The real run of the CLI, front of the real GitHub client.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    run_with(&mut GhCli, args.collect())
}

// The unit tests below execute every line of this file, which is the
// workspace's 100 percent line-coverage standard (docs/RUST.md), except the
// `impl GitHub for GhCli` above — the one impl they cannot, marked so there.
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    /// A recording fake: serves canned answers by (path, jq) and records
    /// every call as `GET <path> --jq <jq>`, so a test can assert every
    /// recorded request is a read — the only kind the trait has.
    struct Fake {
        answers: HashMap<(String, String), Result<String, ReadError>>,
        calls: Vec<String>,
    }

    impl GitHub for Fake {
        fn get(&mut self, path: &str, jq: &str) -> Result<String, ReadError> {
            self.calls.push(format!("GET {path} --jq {jq}"));
            self.answers
                .get(&(path.to_string(), jq.to_string()))
                .cloned()
                .unwrap_or_else(|| Err(ReadError::Unreadable(format!("no canned {path}"))))
        }
    }

    fn canned(answers: Vec<(&str, &str, Result<String, ReadError>)>) -> Fake {
        Fake {
            answers: answers
                .into_iter()
                .map(|(path, jq, answer)| ((path.to_string(), jq.to_string()), answer))
                .collect(),
            calls: Vec::new(),
        }
    }

    /// The fake of one whole working ticket: every read the wrapper makes,
    /// answered from the given fields.
    fn fake_for(
        number: u64,
        title: &str,
        state: &str,
        body: &str,
        subs: &str,
        blocked: &str,
    ) -> Fake {
        let issue = format!("repos/o/r/issues/{number}");
        let sub_issues = format!("{issue}/sub_issues");
        let blocked_by = format!("{issue}/dependencies/blocked_by");
        canned(vec![
            (&issue, TITLE_JQ, Ok(format!("{title}\n"))),
            (&issue, STATE_JQ, Ok(format!("{state}\n"))),
            (&issue, BODY_JQ, Ok(format!("{body}\n"))),
            (&sub_issues, LIST_JQ, Ok(subs.to_string())),
            (&blocked_by, LIST_JQ, Ok(blocked.to_string())),
        ])
    }

    /// The OUT_FILE path of one test: unique under the temp dir.
    fn fixture(name: &str) -> PathBuf {
        let out = std::env::temp_dir().join(format!("ticket-export-{name}-{}", std::process::id()));
        let _ = fs::remove_file(&out);
        out
    }

    /// One command line for `run_with`, the three arguments it expects.
    fn command(repo: &str, number: &str, out: &Path) -> Vec<OsString> {
        vec![repo.into(), number.into(), out.as_os_str().to_os_string()]
    }

    #[test]
    fn a_ticket_with_sub_issues_and_an_open_blocker_exports() {
        let body = "A workflow should start from a ticket.\n\n\
                    ## Acceptance criteria\n\n\
                    - requirement.md holds the ticket, and nothing else.\n\
                    - The wrapper only reads GitHub.\n\n\
                    ## Notes\n\n\
                    No comments or edit history.";
        let expected = format!(
            "# Ticket-to-requirement export wrapper (#355)\n\n\
             ## Body\n\n{body}\n\n\
             ## Acceptance criteria\n\n\
             - requirement.md holds the ticket, and nothing else.\n\
             - The wrapper only reads GitHub.\n\n\
             ## Sub-issues\n\n\
             - #356 Add the wrapper (open)\n\
             - #357 Add the tests (open)\n\n\
             ## Blocked by\n\n\
             - #358 CI pipeline (open)\n"
        );
        let mut gh = fake_for(
            355,
            "Ticket-to-requirement export wrapper",
            "open",
            body,
            "356\topen\tAdd the wrapper\n\n357\topen\tAdd the tests\n",
            "358\topen\tCI pipeline\n",
        );
        // The export itself.
        assert_eq!(export(&mut gh, "o/r", 355), Ok(expected.clone()));
        // Every request the wrapper made is one of the expected GET reads —
        // the trait has no other method, so this list is the whole story.
        assert_eq!(
            gh.calls,
            vec![
                format!("GET repos/o/r/issues/355 --jq {TITLE_JQ}"),
                format!("GET repos/o/r/issues/355 --jq {STATE_JQ}"),
                format!("GET repos/o/r/issues/355 --jq {BODY_JQ}"),
                format!("GET repos/o/r/issues/355/sub_issues --jq {LIST_JQ}"),
                format!("GET repos/o/r/issues/355/dependencies/blocked_by --jq {LIST_JQ}"),
            ]
        );
        // The CLI writes the same file and exits 0.
        let out = fixture("exports");
        let fresh = fake_for(
            355,
            "Ticket-to-requirement export wrapper",
            "open",
            body,
            "356\topen\tAdd the wrapper\n\n357\topen\tAdd the tests\n",
            "358\topen\tCI pipeline\n",
        );
        assert_eq!(run_with(&mut fresh, command("o/r", "355", &out)), 0);
        assert_eq!(fs::read_to_string(&out), Ok(expected));
    }

    #[test]
    fn a_ticket_without_sub_issues_or_blockers_still_names_the_sections() {
        let body = "## Acceptance criteria\n\n- Just export it.";
        let expected = format!(
            "# Small (#7)\n\n## Body\n\n{body}\n\n\
             ## Acceptance criteria\n\n- Just export it.\n\n\
             ## Sub-issues\n\nNone\n\n\
             ## Blocked by\n\nNone\n"
        );
        let mut gh = fake_for(7, "Small", "open", body, "\n", "\n");
        assert_eq!(export(&mut gh, "o/r", 7), Ok(expected));
    }

    #[test]
    fn a_body_without_a_criteria_section_gives_none() {
        let mut gh = fake_for(
            3,
            "Bare",
            "open",
            "No sections, just prose.\n\n\n",
            "60\topen\tAdd a sub-issue\n",
            "\n",
        );
        let expected = "# Bare (#3)\n\n## Body\n\nNo sections, just prose.\n\n\
                        ## Acceptance criteria\n\nNone\n\n\
                        ## Sub-issues\n\n- #60 Add a sub-issue (open)\n\n\
                        ## Blocked by\n\nNone\n"
            .to_string();
        assert_eq!(export(&mut gh, "o/r", 3), Ok(expected));
    }

    #[test]
    fn a_missing_issue_refuses() {
        let out = fixture("missing");
        let mut gh = canned(vec![(
            "repos/o/r/issues/9",
            TITLE_JQ,
            Err(ReadError::NotFound),
        )]);
        assert_eq!(
            export(&mut gh, "o/r", 9),
            Err("issue #9 in o/r: not found".to_string())
        );
        assert_eq!(run_with(&mut gh, command("o/r", "9", &out)), 1);
        assert!(!out.exists());
    }

    #[test]
    fn a_closed_blocker_refuses() {
        let out = fixture("closed-blocker");
        let mut gh = fake_for(
            3,
            "Ticket",
            "open",
            "Body.",
            "\n",
            "358\tclosed\tCI is red\n",
        );
        assert_eq!(
            export(&mut gh, "o/r", 3),
            Err("blocker #358 of issue #3 is closed".to_string())
        );
        assert_eq!(run_with(&mut gh, command("o/r", "3", &out)), 1);
        assert!(!out.exists());
    }

    #[test]
    fn an_unreadable_repository_refuses() {
        let out = fixture("unreadable");
        let mut gh = canned(vec![(
            "repos/o/r/issues/12",
            TITLE_JQ,
            Err(ReadError::Unreadable("gh: no read access".to_string())),
        )]);
        assert_eq!(
            export(&mut gh, "o/r", 12),
            Err("issue #12 in o/r: unreadable: gh: no read access".to_string())
        );
        assert_eq!(run_with(&mut gh, command("o/r", "12", &out)), 1);
        assert!(!out.exists());
    }

    #[test]
    fn an_unparseable_list_line_refuses() {
        let mut gh = fake_for(20, "Weird", "open", "\n", "just one word", "\n");
        assert_eq!(
            export(&mut gh, "o/r", 20),
            Err("issue #20 in o/r: unreadable line \"just one word\"".to_string())
        );
    }

    #[test]
    fn unwritable_out_files_refuse() {
        let out = fixture("write-failure").join("requirement.md");
        let mut gh = fake_for(5, "X", "open", "Body.", "\n", "\n");
        assert_eq!(run_with(&mut gh, command("o/r", "5", &out)), 1);
        assert!(!out.exists());
    }

    #[test]
    fn bad_usage_is_two() {
        let out = fixture("bad-usage");
        let mut gh = canned(vec![]);
        assert_eq!(run_with(&mut gh, vec![]), 2);
        assert_eq!(run_with(&mut gh, vec!["o/r".into()]), 2);
        assert_eq!(run_with(&mut gh, vec!["o/r".into(), "12".into()]), 2);
        assert_eq!(
            run_with(
                &mut gh,
                vec![
                    "o/r".into(),
                    "12".into(),
                    out.clone().into_os_string(),
                    "extra".into()
                ]
            ),
            2
        );
        for number in ["0", "-1", "twelve", "1.5"] {
            assert_eq!(run_with(&mut gh, command("o/r", number, &out)), 2);
        }
        for repo in ["", "/r", "o/", "/r/x", "o/r/x"] {
            assert_eq!(run_with(&mut gh, command(repo, "12", &out)), 2);
        }
        assert!(!out.exists());
        // The real run stays on the same page for bad usage.
        assert_eq!(run(vec!["o/r".into()]), 2);
    }

    #[test]
    fn parse_args_names_each_bad_usage() {
        assert_eq!(
            parse_args(&["o/r".into(), "12".into()]).unwrap_err(),
            "expected exactly 3 arguments, found 2"
        );
        assert_eq!(
            parse_args(&["o".into(), "12".into(), "out.md".into()]).unwrap_err(),
            "\"o\" is not OWNER/REPO"
        );
        assert_eq!(
            parse_args(&["o/r".into(), "0".into(), "out.md".into()]).unwrap_err(),
            "\"0\" is not a positive integer"
        );
        let (repo, number, out) =
            parse_args(&["owner/repo".into(), "42".into(), "out.md".into()]).unwrap();
        assert_eq!(repo, "owner/repo");
        assert_eq!(number, 42);
        assert_eq!(out, OsString::from("out.md"));
        assert_eq!(positive("007"), Some(7));
        assert_eq!(positive("0"), None);
        assert_eq!(positive("nope"), None);
    }
}
