//! Writes requirement.md from one GitHub issue, its sub-issues and its
//! blockers, with read access only (issue #355).
//!
//! Every GitHub answer arrives through [`GitHub`], a GET-only bridge to
//! `gh api`, so a workflow can start from a ticket without any agent holding
//! GitHub write access. The requirement holds the title, body, acceptance
//! criteria, sub-issue list and blocked-by list, and nothing else: no
//! comments, no edit history.
//!
//! Usage: `ticket-export <OWNER/REPO> <ISSUE_NUMBER> <OUT_FILE>`. Exit 0 on
//! success, 1 on a refusal (missing issue, closed blocker, unreadable
//! repository), 2 on bad usage; OUT_FILE is written only after every read
//! succeeded.

use std::ffi::OsString;
use std::fs;
use std::process::Command;

/// The failure of one GitHub read: 404 (the issue or the repository is
/// missing), or anything else that went wrong, with gh's stderr.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    NotFound,
    Unreadable(String),
}

/// A GET-only GitHub REST bridge: one [`get`](GitHub::get) call is one
/// `gh api -X GET <path> --jq <jq>` to a plain-text answer, so a read is the
/// only thing an implementor can do. There is no other method.
pub trait GitHub {
    fn get(&mut self, path: &str, jq: &str) -> Result<String, ReadError>;
}

/// The jq filter for one issue's title.
pub const TITLE_JQ: &str = ".title";
/// The jq filter for one issue's state.
pub const STATE_JQ: &str = ".state";
/// The jq filter for one issue's body ("" when it has none).
pub const BODY_JQ: &str = r#".body // """#;
/// The jq filter turning each entry of a sub-issue or blocked-by list into
/// one `number\tstate\ttitle` line.
pub const LIST_JQ: &str = r#".[] | "\(.number)\t\(.state)\t\(.title)""#;

const CRITERIA_HEADING: &str = "## Acceptance criteria";

/// The real bridge to the `gh` CLI. This one impl is the only code the unit
/// tests do not execute: they drive the trait through a recording fake
/// instead, so it stays a few lines.
pub struct GhCli;

impl GitHub for GhCli {
    fn get(&mut self, path: &str, jq: &str) -> Result<String, ReadError> {
        let out = Command::new("gh")
            .args(["api", "-X", "GET", path, "--jq", jq])
            .output()
            .map_err(|e| ReadError::Unreadable(e.to_string()))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
            if stderr.contains("404") || stderr.contains("Not Found") {
                Err(ReadError::NotFound)
            } else {
                Err(ReadError::Unreadable(stderr))
            }
        }
    }
}

/// Strips the one newline `gh api --jq` ends every plain-text answer with.
fn line(reply: &str) -> &str {
    reply.strip_suffix('\n').unwrap_or(reply)
}

/// The `number\tstate\ttitle` entries of one list answer.
fn entries(reply: &str) -> Vec<(String, String, String)> {
    line(reply)
        .split('\n')
        .filter_map(|l| {
            let (number, rest) = l.split_once('\t')?;
            let (state, title) = rest.split_once('\t')?;
            Some((number.to_string(), state.to_string(), title.to_string()))
        })
        .collect()
}

/// The `- #<n> <title> (<state>)` lines of one list, "" when it is empty.
fn items(list: &[(String, String, String)]) -> String {
    list.iter()
        .map(|(number, state, title)| format!("- #{number} {title} ({state})"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The lines under `## Acceptance criteria` up to the next `## ` heading,
/// "" when the body has no such section.
fn acceptance_criteria(body: &str) -> String {
    let mut lines = body.lines().skip_while(|l| *l != CRITERIA_HEADING);
    if lines.next().is_none() {
        return String::new();
    }
    let mut criteria: Vec<&str> = lines.take_while(|l| !l.starts_with("## ")).collect();
    while criteria.last().is_some_and(|l| l.trim().is_empty()) {
        criteria.pop();
    }
    criteria.join("\n")
}

/// The content, or "None" when there is nothing to list.
fn or_none(content: String) -> String {
    if content.is_empty() {
        "None".to_string()
    } else {
        content
    }
}

/// requirement.md: exactly these sections in order, and nothing else.
fn document(
    number: u64,
    title: &str,
    body: &str,
    criteria: &str,
    subs: &str,
    blocked: &str,
) -> String {
    let mut blocks = vec![format!("# {title} (#{number})"), "## Body".to_string()];
    if !body.is_empty() {
        blocks.push(body.to_string());
    }
    for (heading, content) in [
        ("## Acceptance criteria", criteria),
        ("## Sub-issues", subs),
        ("## Blocked by", blocked),
    ] {
        blocks.push(heading.to_string());
        blocks.push(or_none(content.to_string()));
    }
    format!("{}\n", blocks.join("\n\n"))
}

/// One GitHub read, with the refusal message its failure earns.
fn get(gh: &mut dyn GitHub, path: &str, jq: &str, what: &str) -> Result<String, String> {
    gh.get(path, jq).map_err(|e| match e {
        ReadError::NotFound => format!("{what} not found"),
        ReadError::Unreadable(s) => format!("cannot read {what}: {s}"),
    })
}

/// Reads the issue, its sub-issues and its blocked-by dependencies through
/// the bridge and returns requirement.md, or the refusal message.
pub fn export(gh: &mut dyn GitHub, repo: &str, number: u64) -> Result<String, String> {
    let base = format!("repos/{repo}/issues/{number}");
    let issue = format!("issue #{number} in {repo}");
    let title = get(gh, &base, TITLE_JQ, &issue)?;
    let _state = get(gh, &base, STATE_JQ, &issue)?;
    let body = get(gh, &base, BODY_JQ, &issue)?;
    let subs = get(
        gh,
        &format!("{base}/sub_issues"),
        LIST_JQ,
        &format!("the sub-issues of {issue}"),
    )?;
    let blocked = get(
        gh,
        &format!("{base}/dependencies/blocked_by"),
        LIST_JQ,
        &format!("the blockers of {issue}"),
    )?;
    let subs = entries(&subs);
    let blocked = entries(&blocked);
    for (blocker, state, _) in &blocked {
        if state == "closed" {
            return Err(format!("{issue} is blocked by closed issue #{blocker}"));
        }
    }
    let body = line(&body);
    Ok(document(
        number,
        line(&title),
        body,
        &acceptance_criteria(body),
        &items(&subs),
        &items(&blocked),
    ))
}

/// `OWNER/REPO`, not anything else.
fn valid_repo(repo: &str) -> bool {
    match repo.split_once('/') {
        Some((owner, name)) => !owner.is_empty() && !name.is_empty() && !name.contains('/'),
        None => false,
    }
}

/// A positive-integer issue number.
fn positive(number: &str) -> Option<u64> {
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse::<u64>().ok().filter(|&n| n > 0)
}

/// Runs one command line against the bridge and returns its exit code:
/// 0 success, 1 a refusal, 2 bad usage. Success prints to stdout, problems
/// to stderr, and OUT_FILE is written only after every read succeeded.
pub fn run_with(gh: &mut dyn GitHub, args: Vec<OsString>) -> i32 {
    let args: Vec<OsString> = args.into_iter().collect();
    if args.len() != 3 {
        eprintln!("usage: ticket-export <OWNER/REPO> <ISSUE_NUMBER> <OUT_FILE>");
        return 2;
    }
    let repo = args[0].to_string_lossy().into_owned();
    if !valid_repo(&repo) {
        eprintln!("ticket-export: <OWNER/REPO> must be owner/name, not {repo:?}");
        return 2;
    }
    let shown = args[1].to_string_lossy().into_owned();
    let Some(number) = positive(&shown) else {
        eprintln!("ticket-export: <ISSUE_NUMBER> must be a positive integer, not {shown:?}");
        return 2;
    };
    let out = args[2].clone();
    let shown = out.to_string_lossy().into_owned();
    match export(gh, &repo, number) {
        Ok(markdown) => match fs::write(&out, markdown) {
            Ok(()) => {
                println!("ticket-export: wrote {shown}");
                0
            }
            Err(e) => {
                eprintln!("ticket-export: cannot write {shown}: {e}");
                1
            }
        },
        Err(message) => {
            eprintln!("ticket-export: {message}");
            1
        }
    }
}

/// Runs one command line with the real bridge.
pub fn run<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
    run_with(&mut GhCli, args.into_iter().collect())
}

// The unit tests below execute every line of this file except the GhCli impl
// above: that impl is the one code they cannot, because it shells out to `gh`.
#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "acme/kit";
    const N: u64 = 355;

    /// A recording fake: answers by (path, jq) and records every call. The
    /// trait has no non-GET method, so the call list is everything that can
    /// happen on GitHub.
    struct Fake {
        answers: Vec<((String, String), Result<String, ReadError>)>,
        calls: Vec<(String, String)>,
    }

    impl GitHub for Fake {
        fn get(&mut self, path: &str, jq: &str) -> Result<String, ReadError> {
            self.calls.push((path.to_string(), jq.to_string()));
            for ((p, j), answer) in &self.answers {
                if p == path && j == jq {
                    return answer.clone();
                }
            }
            panic!("no canned answer for GET {path} with jq {jq}");
        }
    }

    /// A fake serving the ticket (title, state, body), its sub-issues and its
    /// blocked-by list; each answer plain text, as `gh api --jq` prints it.
    fn canned(
        title: Result<&str, ReadError>,
        state: &str,
        body: &str,
        subs: &str,
        blocked: &str,
    ) -> Fake {
        let issue = format!("repos/{REPO}/issues/{N}");
        let answers = vec![
            (
                (issue.clone(), TITLE_JQ.to_string()),
                title.map(str::to_string),
            ),
            ((issue.clone(), STATE_JQ.to_string()), Ok(state.to_string())),
            ((issue.clone(), BODY_JQ.to_string()), Ok(body.to_string())),
            (
                (format!("{issue}/sub_issues"), LIST_JQ.to_string()),
                Ok(subs.to_string()),
            ),
            (
                (
                    format!("{issue}/dependencies/blocked_by"),
                    LIST_JQ.to_string(),
                ),
                Ok(blocked.to_string()),
            ),
        ];
        Fake {
            answers,
            calls: Vec::new(),
        }
    }

    /// A fixture ticket with two sub-issues and one open blocker.
    fn happy() -> Fake {
        canned(
            Ok("Ticket-to-requirement export wrapper\n"),
            "open\n",
            "Exports issue #355.\n\n## Acceptance criteria\n- every read is a GET\n- no comments, no edit history\n",
            "101\topen\tFirst sub\n102\topen\tSecond sub\n",
            "201\topen\tOnly blocker\n",
        )
    }

    fn expected_calls() -> Vec<(String, String)> {
        let issue = format!("repos/{REPO}/issues/{N}");
        vec![
            (issue.clone(), TITLE_JQ.to_string()),
            (issue.clone(), STATE_JQ.to_string()),
            (issue.clone(), BODY_JQ.to_string()),
            (format!("{issue}/sub_issues"), LIST_JQ.to_string()),
            (
                format!("{issue}/dependencies/blocked_by"),
                LIST_JQ.to_string(),
            ),
        ]
    }

    /// A unique OUT_FILE path for this test, absent before the test starts.
    fn out_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ticket-export-{tag}-{}", std::process::id()))
    }

    /// Runs the fixture command line against a fake and returns its exit
    /// code with the OUT_FILE path.
    fn run_fake(fake: &mut Fake, tag: &str) -> (i32, std::path::PathBuf) {
        let out = out_path(tag);
        let _ = fs::remove_file(&out);
        let args = vec![
            OsString::from(REPO),
            OsString::from(N.to_string()),
            out.clone().into_os_string(),
        ];
        (run_with(fake, args), out)
    }

    #[test]
    fn fixture_becomes_requirement_md_through_reads_only() {
        let mut fake = happy();
        let (code, out) = run_fake(&mut fake, "fixture");
        assert_eq!(code, 0);
        let expected = "# Ticket-to-requirement export wrapper (#355)\n\n## Body\n\nExports issue #355.\n\n## Acceptance criteria\n- every read is a GET\n- no comments, no edit history\n\n## Acceptance criteria\n\n- every read is a GET\n- no comments, no edit history\n\n## Sub-issues\n\n- #101 First sub (open)\n- #102 Second sub (open)\n\n## Blocked by\n\n- #201 Only blocker (open)\n";
        assert_eq!(fs::read_to_string(&out).unwrap(), expected);
        // The trait has no non-GET method, so exactly these calls happened.
        assert_eq!(fake.calls, expected_calls());
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn missing_issue_refuses_without_writing() {
        let mut probe = canned(Err(ReadError::NotFound), "", "", "", "");
        let refused = export(&mut probe, REPO, N).unwrap_err();
        assert_eq!(refused, format!("issue #{N} in {REPO} not found"));
        assert_eq!(probe.calls.len(), 1);
        let mut fake = canned(Err(ReadError::NotFound), "", "", "", "");
        let (code, out) = run_fake(&mut fake, "missing");
        assert_eq!(code, 1);
        assert!(!out.exists());
    }

    #[test]
    fn closed_blocker_refuses_without_writing() {
        let mut probe = canned(
            Ok("Blocked upstream\n"),
            "open\n",
            "\n",
            "101\topen\tFirst sub\n",
            "201\tclosed\tBroken blocker\n",
        );
        let refused = export(&mut probe, REPO, N).unwrap_err();
        assert_eq!(
            refused,
            format!("issue #{N} in {REPO} is blocked by closed issue #201")
        );
        let mut fake = canned(
            Ok("Blocked upstream\n"),
            "open\n",
            "\n",
            "101\topen\tFirst sub\n",
            "201\tclosed\tBroken blocker\n",
        );
        let (code, out) = run_fake(&mut fake, "blocker");
        assert_eq!(code, 1);
        assert!(!out.exists());
    }

    #[test]
    fn unreadable_repository_refuses_without_writing() {
        fn denied() -> ReadError {
            ReadError::Unreadable("gh: no access".into())
        }
        let mut probe = canned(Err(denied()), "", "", "", "");
        let refused = export(&mut probe, REPO, N).unwrap_err();
        assert_eq!(
            refused,
            format!("cannot read issue #{N} in {REPO}: gh: no access")
        );
        let mut fake = canned(Err(denied()), "", "", "", "");
        let (code, out) = run_fake(&mut fake, "unreadable");
        assert_eq!(code, 1);
        assert!(!out.exists());
    }

    #[test]
    fn absent_sections_render_none() {
        let mut fake = canned(Ok("Bare ticket\n"), "open\n", "\n", "", "");
        let markdown = export(&mut fake, REPO, N).unwrap();
        assert_eq!(
            markdown,
            "# Bare ticket (#355)\n\n## Body\n\n## Acceptance criteria\n\nNone\n\n## Sub-issues\n\nNone\n\n## Blocked by\n\nNone\n"
        );
    }

    #[test]
    fn unwritable_out_file_exits_one() {
        let dir = out_path("unwritable");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut fake = happy();
        let code = run_with(
            &mut fake,
            vec![
                OsString::from(REPO),
                OsString::from(N.to_string()),
                dir.clone().into_os_string(),
            ],
        );
        assert_eq!(code, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_usage_exits_two() {
        fn arg(s: &str) -> OsString {
            OsString::from(s)
        }
        let mut fake = happy();
        let out = out_path("usage");
        let _ = fs::remove_file(&out);
        let file = || out.clone().into_os_string();
        let cases = vec![
            vec![],
            vec![arg(REPO)],
            vec![arg(REPO), arg("355")],
            vec![arg(REPO), arg("355"), file(), arg("extra")],
            vec![arg("acmegit"), arg("355"), file()],
            vec![arg("acme/kit/extra"), arg("355"), file()],
            vec![arg("acme/"), arg("355"), file()],
            vec![arg("/kit"), arg("355"), file()],
            vec![arg(REPO), arg("0"), file()],
            vec![arg(REPO), arg("-1"), file()],
            vec![arg(REPO), arg("4x"), file()],
            vec![arg(REPO), arg(""), file()],
        ];
        for args in cases {
            assert_eq!(run_with(&mut fake, args), 2);
        }
        assert_eq!(run(vec![arg(REPO)]), 2);
        assert!(!out.exists());
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn criteria_stop_at_the_next_heading() {
        assert_eq!(
            acceptance_criteria("## Acceptance criteria\n- a\n\n## Notes\nn"),
            "- a"
        );
        assert_eq!(acceptance_criteria("## Acceptance criteria\n- a"), "- a");
        assert_eq!(acceptance_criteria("no section"), "");
    }
}
