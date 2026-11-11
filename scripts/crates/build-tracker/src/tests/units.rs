//! Unit tests of the parts the scenarios reach only in passing: the JSON
//! reader and writer, JavaScript text rules, the adapters' failure paths and
//! the stamp.

use super::scenarios::{expected, read_json, Replay, FIXED_NOW};
use crate::adapters::clock::SystemClock;
use crate::adapters::fs::{fail, write_then_rename, SystemFiles};
use crate::adapters::gh::GhCliIssues;
use crate::adapters::git::{parse_remote, GitRepositories};
use crate::adapters::process::SystemProcess;
use crate::json::{compact, number_text, object, parse, pretty, Object, Value};
use crate::ports::{Clock, Files, FsError, IssueRecord, Issues, Process, ProcessOutput, Repositories, RepositoryLocation, Runtime};
use crate::spec::*;
use crate::state_file::stamp;
use crate::text::*;
use std::cell::RefCell;
use std::collections::HashMap;

fn repo(host: &str, owner: &str, name: &str) -> RepositoryName {
    repository_name(host, owner, name)
}

#[test]
fn json_numbers_print_as_javascript_prints_them() {
    let cases = [
        ("0", "0"),
        ("-0", "0"),
        ("1", "1"),
        ("1.5", "1.5"),
        ("100", "100"),
        ("1e2", "100"),
        ("0.1", "0.1"),
        ("-12.25", "-12.25"),
        ("123456789012345680000", "123456789012345680000"),
        ("1e21", "1e+21"),
        ("1.5e21", "1.5e+21"),
        ("1.5e-7", "1.5e-7"),
        ("1e-7", "1e-7"),
        ("0.000001", "0.000001"),
        ("1e999", "null"),
    ];
    for (input, output) in cases {
        let value = parse(input).expect(input);
        assert_eq!(compact(&value), output, "{input}");
    }
    assert_eq!(number_text(f64::NAN), "null");
}

#[test]
fn json_strings_escape_as_stringify_does() {
    let value = parse(r#""a\"b\\c\/d\b\f\n\r\t\u0001\u007f\u00e9\ud83d\ude00\u2028""#).expect("string");
    assert_eq!(compact(&value), "\"a\\\"b\\\\c/d\\b\\f\\n\\r\\t\\u0001\u{7f}é😀\u{2028}\"");
    // A lone surrogate cannot be held; it reads as the replacement character, a pair that is not one too.
    assert_eq!(parse(r#""\ud83d""#), Ok(Value::str("\u{fffd}")));
    assert_eq!(parse(r#""\ud83dx""#), Ok(Value::str("\u{fffd}x")));
    assert_eq!(parse(r#""\ud83d\u0041""#), Ok(Value::str("\u{fffd}A")));
    assert_eq!(parse(r#""\ude00""#), Ok(Value::str("\u{fffd}")));
}

#[test]
fn json_objects_keep_javascript_key_order() {
    let value = parse(r#"{"b":1,"10":2,"a":3,"2":4,"4294967295":5,"01":6,"b":7}"#).expect("object");
    assert_eq!(compact(&value), r#"{"2":4,"10":2,"b":7,"a":3,"4294967295":5,"01":6}"#);
    let mut members = Object::new();
    members.set("z", Value::Null);
    members.set("y", Value::Bool(true));
    members.remove("z");
    assert_eq!(pretty(&Value::Obj(members)), "{\n  \"y\": true\n}");
    assert_eq!(pretty(&object(vec![("a", Value::Arr(vec![])), ("b", Value::Obj(Object::new()))])), "{\n  \"a\": [],\n  \"b\": {}\n}");
    assert_eq!(Value::Null.as_object(), None);
    assert_eq!(Value::Null.as_str(), None);
}

#[test]
fn json_syntax_errors_read_as_v8_words_them() {
    let cases = [
        ("", "Unexpected end of JSON input"),
        ("{", "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
        ("{\"a\"", "Expected ':' after property name in JSON at position 4 (line 1 column 5)"),
        ("{\"a\":1,", "Expected double-quoted property name in JSON at position 7 (line 1 column 8)"),
        ("[1 2]", "Expected ',' or ']' after array element in JSON at position 3 (line 1 column 4)"),
        ("not json {", "Unexpected token 'o', \"not json {\" is not valid JSON"),
        ("[1,]", "Unexpected token ']', \"[1,]\" is not valid JSON"),
        ("\"abc", "Unterminated string in JSON at position 4 (line 1 column 5)"),
        ("\"\\", "Unexpected end of JSON input"),
        ("\"\\x\"", "Bad escaped character in JSON at position 2 (line 1 column 3)"),
        ("\"\\u12\"", "Bad Unicode escape in JSON at position 5 (line 1 column 6)"),
        ("\"\u{1}\"", "Bad control character in string literal in JSON at position 1 (line 1 column 2)"),
        ("-", "No number after minus sign in JSON at position 1 (line 1 column 2)"),
        ("1.", "Unterminated fractional number in JSON at position 2 (line 1 column 3)"),
        ("1e+", "Exponent part is missing a number in JSON at position 3 (line 1 column 4)"),
        ("01", "Unexpected number in JSON at position 1 (line 1 column 2)"),
        ("[] x", "Unexpected non-whitespace character after JSON at position 3 (line 1 column 4)"),
        ("{\"a\":\n1 x}", "Expected ',' or '}' after property value in JSON at position 8 (line 2 column 3)"),
        ("{\"a\":\r\n1 x}", "Expected ',' or '}' after property value in JSON at position 9 (line 2 column 3)"),
        ("{\"a\":\r1 x}", "Expected ',' or '}' after property value in JSON at position 8 (line 2 column 3)"),
        ("trux", "Unexpected token 'x', \"trux\" is not valid JSON"),
        (&"x".repeat(30), "Unexpected token 'x', \"xxxxxxxxxx\"... is not valid JSON"),
        (&format!("[{}x", "1,".repeat(15)), "Unexpected token 'x', ...\"1,1,1,1,1,x\" is not valid JSON"),
        (&format!("[{}x{}", "1,".repeat(15), ",2".repeat(15)), "Unexpected token 'x', ...\"1,1,1,1,1,x,2,2,2,2,\"... is not valid JSON"),
    ];
    for (input, message) in cases {
        assert_eq!(parse(input), Err(message.to_string()), "{input:?}");
    }
    assert_eq!(parse(" [ true , false , null , {} ] "), Ok(Value::Arr(vec![Value::Bool(true), Value::Bool(false), Value::Null, Value::Obj(Object::new())])));
}

#[test]
fn text_helpers_follow_javascript() {
    assert!(is_space('\u{feff}') && is_space('\u{a0}') && !is_space('\u{85}') && !is_space('a'));
    assert_eq!(trim("\u{feff} x \u{3000}"), "x");
    assert_eq!(first_line("\n  \n  second \nthird"), Some("second".to_string()));
    assert_eq!(first_line(" \n "), None);
    assert_eq!(encode_uri_component("a b/é-_.!~*'()"), "a%20b%2F%C3%A9-_.!~*'()");
    assert_eq!(fnv_hash("docs/spec.md"), "2d77c443");
    assert_eq!(fnv_hash("😀"), "cb31c4b8");
    assert_eq!(utf16_cmp("\u{ffee}", "😀"), std::cmp::Ordering::Greater);
    assert_eq!(normalize("/a//b/./c/../d/.."), "/a/b");
    assert_eq!(normalize("/.."), "/");
    assert_eq!(join(&["/a/", "b", "../c"]), "/a/c");
    assert_eq!(resolve("/work", "x/../y"), "/work/y");
    assert_eq!(resolve("/work", "/abs//path/"), "/abs/path");
    assert_eq!((basename("/a/b/"), basename("/"), dirname("/a/b/"), dirname("/a"), dirname("/")), ("b", "", "/a", "/", "/"));
    assert_eq!((without_md("a.md"), without_md("a.MD"), without_md("README"), without_md("é"), without_md(".md")), ("a", "a", "README", "é", ""));
    assert_eq!(parse_number("12"), Some(12));
    assert_eq!(parse_number("99999999999999999999999"), None);
}

#[test]
fn spec_arguments_read_as_the_build_tracker_reads_them() {
    let widgets = repo("github.com", "acme", "widgets");
    let issue = |number, repository| SpecArgument::Issue { number, repository };
    assert_eq!(parse_spec_argument(" #12 "), issue(12, None));
    assert_eq!(parse_spec_argument("12"), issue(12, None));
    assert_eq!(parse_spec_argument("acme/widgets#12"), issue(12, Some(widgets.clone())));
    assert_eq!(parse_spec_argument("GHE.example.com/acme/widgets#12"), issue(12, Some(repo("ghe.example.com", "acme", "widgets"))));
    assert_eq!(parse_spec_argument("HTTPS://github.com/acme/widgets/ISSUES/12/x?y#z"), issue(12, Some(widgets.clone())));
    assert_eq!(parse_spec_argument("http://github.com/acme/widgets/issues/12"), issue(12, Some(widgets)));
    for path in [
        "0", "#0", "99999999999999999999999", "docs/spec.md", "acme/widgets#0", "acme/widgets#07", "acme/widgets#1#2", "a/b/c/d#1", "a//b#1", "a b/c#1", "acme/widgets#99999999999999999999999",
        "https://github.com/acme/widgets/pull/12", "https://github.com/acme/widgets/issues/12 x", "https://github.com/acme/widgets/issues/12x", "https://github.com/acme/widgets/issues/0",
        "ftp://github.com/acme/widgets/issues/12", "https://github.com/acme/widgets/issues", "https://github.com/acme/widgets/commits/12", "https://github.com//widgets/issues/12",
        "https://github.com/acme/widgets/issues/",
    ] {
        assert_eq!(parse_spec_argument(path), SpecArgument::Path(path.to_string()), "{path}");
    }
    let pull = parse_issue_url("https://github.com/acme/widgets/pull/3").expect("pull");
    assert!(pull.pull);
    assert_eq!(parse_issue_url("https://github.com/acme/widgets/issues/3#frag").map(|url| url.reference.number), Some(3));
}

#[test]
fn spec_names_and_places() {
    let widgets = repo("github.com", "Acme", "Widgets");
    assert!(same_repository(&widgets, &repo("GITHUB.com", "acme", "widgets")));
    assert!(!same_repository(&widgets, &repo("gitlab.com", "acme", "widgets")));
    assert_eq!(issue_reference_text(&widgets, 3), "Acme/Widgets#3");
    assert_eq!(issue_reference_text(&repo("ghe.io", "a", "b"), 3), "ghe.io/a/b#3");
    assert_eq!(spec_text(&ResolvedSpec::Local { repository: widgets.clone(), path: "docs/x.md".into() }), "docs/x.md");
    assert_eq!(local_repository_name("/home/me/Projects/tool"), repo("local", "Projects", "tool"));
    assert_eq!(local_repository_name("/"), repo("local", "local", "local"));
    assert_eq!(spec_slug("Docs/V1.2 spec.md"), format!("docs-v1-2-spec-md-{}", fnv_hash("Docs/V1.2 spec.md")));
    assert_eq!(spec_slug("..."), format!("-{}", fnv_hash("...")));
    assert_eq!(file_link(&TicketFiles::Disk, "a"), None);
    assert_eq!(file_link(&ticket_files(Some(&widgets)), "docs/a b.md"), Some("https://github.com/Acme/Widgets/blob/HEAD/docs/a%20b.md".to_string()));
    assert_eq!(ticket_files(Some(&repo("gitlab.com", "a", "b"))), TicketFiles::Disk);
    assert_eq!(ticket_files(None), TicketFiles::Disk);
    let env = |pairs: &[(&str, &str)]| -> HashMap<String, String> { pairs.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect() };
    assert_eq!(state_base(&env(&[("XDG_STATE_HOME", "/x/"), ("HOME", "/h")])), Some("/x/agent-kit/build-tracker".to_string()));
    assert_eq!(state_base(&env(&[("XDG_STATE_HOME", "x"), ("HOME", "/h")])), Some("/h/.local/state/agent-kit/build-tracker".to_string()));
    assert_eq!(state_base(&env(&[("HOME", "h")])), None);
    let place = state_location("/s", &ResolvedSpec::Github { repository: widgets, number: 4 });
    assert_eq!((place.state.as_str(), place.page.as_str()), ("/s/github.com/acme/widgets/4/state.json", "/s/github.com/acme/widgets/4/page.html"));
}

#[test]
fn the_stamp_is_utc_to_the_minute() {
    assert_eq!(stamp(0), "1970-01-01 00:00 UTC");
    assert_eq!(stamp(-1), "1969-12-31 23:59 UTC");
    assert_eq!(stamp(FIXED_NOW), "2026-11-03 12:00 UTC");
    assert_eq!(stamp(951_782_400 + 86_399), "2000-02-29 23:59 UTC");
    assert_eq!(stamp(1_709_164_800), "2024-02-29 00:00 UTC");
    assert_eq!(stamp(4_102_444_799), "2099-12-31 23:59 UTC");
    assert_eq!(stamp(1_700_000_059), "2023-11-14 22:14 UTC");
}

/// The page's stamp is the one thing that differs between runs; the goldens are made with the clock fixed, so a run on
/// the real clock is compared with its stamp replaced.
fn normalise_stamp(page: &str) -> String {
    let at = page.find("updated ").expect("a stamp") + "updated ".len();
    let end = at + page[at..].find(" UTC").expect("a UTC stamp") + " UTC".len();
    format!("{}@STAMP@{}", &page[..at], &page[end..])
}

#[test]
fn a_run_on_the_real_clock_matches_the_golden_but_for_its_stamp() {
    let replay = Replay::new("render-hand");
    let golden = read_json(&replay.dir.join("expected.json"));
    let step = &match replay.scenario.as_object().and_then(|scenario| scenario.get("steps")) {
        Some(Value::Arr(steps)) => steps.clone(),
        _ => Vec::new(),
    }[0];
    let before = stamp(SystemClock.now());
    let got = replay.step_at(step, Box::new(SystemClock));
    let after = stamp(SystemClock.now());
    let want = expected(&golden, 0);
    let page = |files: &std::collections::BTreeMap<String, Option<String>>| files.get("out/hand.html").cloned().flatten().expect("the page");
    let (got_page, want_page) = (page(&got.files), page(&want.files));
    assert!(got_page.contains("updated 2") && !got_page.contains(&stamp(FIXED_NOW)), "the real clock stamps the page");
    let seen = &got_page[got_page.find("updated ").map_or(0, |at| at + 8)..][..20];
    assert!(seen == before || seen == after, "{seen} is neither {before} nor {after}");
    assert_eq!(normalise_stamp(&got_page), normalise_stamp(&want_page));
    assert_eq!(got.stdout, want.stdout);
}

struct FakeProcess {
    replies: RefCell<Vec<Result<ProcessOutput, String>>>,
    calls: RefCell<Vec<String>>,
}

impl FakeProcess {
    fn new(replies: Vec<Result<ProcessOutput, String>>) -> FakeProcess {
        FakeProcess { calls: RefCell::new(Vec::new()), replies: RefCell::new(replies.into_iter().rev().collect()) }
    }
}

impl Process for FakeProcess {
    fn exec(&self, command: &str, args: &[String]) -> Result<ProcessOutput, String> {
        self.calls.borrow_mut().push(format!("{command} {}", args.join(" ")));
        self.replies.borrow_mut().pop().unwrap_or_else(|| Err("no reply".to_string()))
    }
}

fn out(stdout: &str) -> Result<ProcessOutput, String> {
    Ok(ProcessOutput { exit_code: 0, stderr: String::new(), stdout: stdout.to_string() })
}

fn failed(code: i32, stdout: &str, stderr: &str) -> Result<ProcessOutput, String> {
    Ok(ProcessOutput { exit_code: code, stderr: stderr.to_string(), stdout: stdout.to_string() })
}

#[test]
fn gh_reads_issues_through_gh_api() {
    let widgets = repo("github.com", "acme", "widgets");
    let gh = |replies| GhCliIssues { process: FakeProcess::new(replies) };
    let reader = gh(vec![out(r#"{"number":10,"title":"Spec","state":"closed","html_url":"https://ghe.io/o/r/pull/10"}"#)]);
    let issue = reader.issue(&widgets, 10).expect("issue");
    assert_eq!((issue.number, issue.title.as_str(), issue.closed, issue.repository), (10, "Spec", true, repo("ghe.io", "o", "r")));
    assert_eq!(reader.process.calls.borrow()[0], "gh api --hostname github.com repos/acme/widgets/issues/10");
    let reader = gh(vec![out(r#"[[{"number":1},{"number":2,"title":"two","state":"open"}],[{"number":3}]]"#)]);
    let subs = reader.sub_issues(&widgets, 10).expect("sub-issues");
    assert_eq!(subs.iter().map(|sub| sub.number).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(reader.process.calls.borrow()[0], "gh api --hostname github.com --paginate --slurp repos/acme/widgets/issues/10/sub_issues?per_page=100");
    let reader = gh(vec![out("[[]]")]);
    assert_eq!(reader.blocked_by(&widgets, 10), Ok(Vec::new()));
    assert_eq!(reader.process.calls.borrow()[0], "gh api --hostname github.com --paginate --slurp repos/acme/widgets/issues/10/dependencies/blocked_by?per_page=100");
    let errors = [
        (failed(1, "", "  \nfirst\nsecond"), "gh api repos/acme/widgets/issues/10 exited 1: first"),
        (failed(4, "from stdout\n", ""), "gh api repos/acme/widgets/issues/10 exited 4: from stdout"),
        (failed(2, "", ""), "gh api repos/acme/widgets/issues/10 exited 2: exit code 2"),
        (out("nope"), "gh api printed something that is not an issue"),
        (out("[]"), "gh api printed something that is not an issue"),
        (out(r#"{"number":"1"}"#), "gh api printed something that is not an issue"),
        (Err("cannot run gh: spawn gh ENOENT".to_string()), "cannot run gh: spawn gh ENOENT"),
    ];
    for (reply, message) in errors {
        assert_eq!(gh(vec![reply]).issue(&widgets, 10).map(|issue| issue.number), Err(message.to_string()));
    }
    for body in ["nope", "{}", "[1]", r#"[[{"title":"x"}]]"#, "[{\"number\":1}]"] {
        assert_eq!(gh(vec![out(body)]).sub_issues(&widgets, 10), Err("gh api printed something that is not a list of issues".to_string()), "{body}");
    }
    assert_eq!(gh(vec![failed(1, "", "boom")]).sub_issues(&widgets, 10), Err("gh api repos/acme/widgets/issues/10/sub_issues?per_page=100 exited 1: boom".to_string()));
}

fn paths(root: &str, common: &str, prefix: &str) -> Result<ProcessOutput, String> {
    out(&format!("{root}\n{common}\n{prefix}\n"))
}

#[test]
fn git_locates_a_directory_in_its_repository() {
    let git = |replies| GitRepositories { process: FakeProcess::new(replies) };
    let finder = git(vec![paths("/r", "/r/.git", "docs/"), out("remote.origin.url git@github.com:acme/widgets.git\n")]);
    let place = finder.locate("/r/docs").expect("located").expect("in a repository");
    assert_eq!((place.root.as_str(), place.prefix.as_str(), place.main_root.as_str()), ("/r", "docs/", "/r"));
    assert_eq!(place.remote, Some(repo("github.com", "acme", "widgets")));
    assert_eq!(finder.process.calls.borrow()[0], "git -C /r/docs rev-parse --path-format=absolute --show-toplevel --git-common-dir --show-prefix");
    assert_eq!(finder.process.calls.borrow()[1], r"git -C /r/docs config --get-regexp ^remote\..*\.url$");
    let linked = git(vec![paths("/wt/feature", "/main/widgets/.git", ""), failed(1, "", "")]).locate("/wt/feature").expect("located").expect("in a repository");
    assert_eq!((linked.main_root.as_str(), linked.prefix.as_str(), linked.remote), ("/main/widgets", "", None));
    let bare = git(vec![paths("/r", "/srv/widgets.git", ""), out("remote.upstream.url https://github.com/a/b\nremote.fork.url nonsense\nremote.odd.url https://github.com/a/odd\u{2028}x\n")]).locate("/r").expect("located").expect("in a repository");
    assert_eq!((bare.main_root.as_str(), bare.remote), ("/srv/widgets.git", Some(repo("github.com", "a", "b"))));
    let chosen = git(vec![paths("/r", "/r/.git", ""), out("remote.fork.url https://github.com/a/fork\nremote.origin.url https://github.com/a/main\n")]).locate("/r");
    assert_eq!(chosen.expect("located").and_then(|place| place.remote), Some(repo("github.com", "a", "main")));
    let short = git(vec![out("/r\n"), out("")]).locate("/r").expect("located").expect("in a repository");
    assert_eq!((short.main_root.as_str(), short.prefix.as_str()), ("", ""));
    assert_eq!(git(vec![failed(128, "", "not a repository")]).locate("/tmp"), Ok(None));
    assert_eq!(git(vec![Err("cannot run git: spawn git ENOENT".to_string())]).locate("/tmp"), Err("cannot run git: spawn git ENOENT".to_string()));
}

#[test]
fn remote_urls_read_in_every_form_git_writes_them() {
    let accepted = [
        ("git@github.com:acme/widgets.git", repo("github.com", "acme", "widgets")),
        ("git@GitHub.com:acme/widgets", repo("github.com", "acme", "widgets")),
        ("https://github.com/acme/widgets.git", repo("github.com", "acme", "widgets")),
        ("HTTPS://token@git.example.org:8443/group/sub/widgets/", repo("git.example.org", "group/sub", "widgets")),
        ("ssh://git@github.com:22/acme/widgets.git/", repo("github.com", "acme", "widgets")),
        ("git+ssh://github.com/acme/widgets", repo("github.com", "acme", "widgets")),
        ("ssh://a@b@host/o/r", repo("b@host", "o", "r")),
    ];
    for (url, name) in accepted {
        assert_eq!(parse_remote(url), Some(name), "{url}");
    }
    for url in [
        "", "nonsense", "git@github.com:widgets.git", "git@github.com:", "@github.com:a/b", "a/b@github.com:a/b", "git@:a/b", "git@git hub:a/b", "git@a/b:a/b", "https://github.com", "https://github.com/",
        "https://github.com/widgets", "https://github.com:x/a/b", "https://github.com:22", "https://:22/a/b", "1://github.com/a/b", "://github.com/a/b", "https://github.com/a/b\nc", "git@github.com:a/b\r",
        "/local/path", "file:///srv/a.git",
    ] {
        assert_eq!(parse_remote(url), None, "{url:?}");
    }
}

#[test]
fn system_files_say_what_went_wrong_as_node_does() {
    let sandbox = super::scenarios::Sandbox::new();
    let dir = sandbox.text();
    let files = SystemFiles { cwd: dir.clone() };
    assert_eq!(files.read_text("missing.txt").map_err(|error| error.code), Err("ENOENT".to_string()));
    let error = files.read_text(".").expect_err("a directory is not text");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("EISDIR", "EISDIR: illegal operation on a directory, read"));
    assert_eq!(files.list_files("missing").expect_err("no folder").message, "ENOENT: no such file or directory, scandir 'missing'");
    files.write_atomic("sub/dir/file.txt", "one").expect("write");
    files.write_atomic("sub/dir/file.txt", "two").expect("replace");
    assert_eq!(files.read_text(&format!("{dir}/sub/dir/file.txt")), Ok("two".to_string()));
    std::fs::create_dir(format!("{dir}/sub/dir/folder")).expect("folder");
    assert_eq!(files.list_files("sub/dir"), Ok(vec!["file.txt".to_string()]));
    // Replacing a folder fails, and leaves no temporary file behind.
    let error = files.write_atomic("sub/dir/folder", "x").expect_err("a folder is not replaced");
    assert_eq!(error.code, "EISDIR");
    assert_eq!(files.list_files("sub/dir"), Ok(vec!["file.txt".to_string()]));
    let error = files.write_atomic("sub/dir/file.txt/inner.txt", "x").expect_err("a file is no folder");
    assert!(error.message.starts_with("EEXIST: file already exists, mkdir") || error.message.starts_with("ENOTDIR: not a directory, mkdir"), "{}", error.message);
    let missing = format!("{dir}/nowhere/tmp");
    assert_eq!(write_then_rename(&missing, &format!("{dir}/x"), "x").map_err(|error| error.message), Err(format!("ENOENT: no such file or directory, open '{missing}'")));
    for (errno, code) in [(2, "ENOENT"), (13, "EACCES"), (17, "EEXIST"), (20, "ENOTDIR"), (21, "EISDIR"), (39, "ENOTEMPTY"), (5, "UNKNOWN")] {
        assert_eq!(fail(&std::io::Error::from_raw_os_error(errno), "open", Some("p")).code, code);
    }
    assert_eq!(fail(&std::io::Error::from_raw_os_error(13), "read", None).message, "EACCES: permission denied, read");
}

#[test]
fn system_process_runs_programs_and_names_the_ones_it_cannot() {
    let echo = SystemProcess { env: vec![("BUILD_TRACKER_TEST".to_string(), "seen".to_string())] };
    let sh = |script: &str| echo.exec("sh", &["-c".to_string(), script.to_string()]).expect("sh runs");
    assert_eq!(sh("printf %s \"$BUILD_TRACKER_TEST\"").stdout, "seen");
    let failing = sh("echo oops >&2; exit 3");
    assert_eq!((failing.exit_code, failing.stderr.as_str()), (3, "oops\n"));
    assert_eq!(sh("kill -9 $$").exit_code, -1);
    assert_eq!(echo.exec("build-tracker-no-such-program", &[]).map(|output| output.exit_code), Err("cannot run build-tracker-no-such-program: spawn build-tracker-no-such-program ENOENT".to_string()));
    let denied = SystemProcess { env: Vec::new() }.exec("/", &[]);
    assert!(denied.expect_err("a folder cannot run").starts_with("cannot run /: spawn / "));
}

#[test]
fn the_command_line_runs_against_the_real_system() {
    let runtime = crate::default_runtime();
    assert!(runtime.cwd.starts_with('/') && runtime.env.contains_key("PATH"));
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    assert_eq!(crate::run_with(["--help".to_string()].into_iter(), &runtime, &mut stdout, &mut stderr), 0);
    assert!(String::from_utf8_lossy(&stdout).starts_with("usage: build-tracker init") && stderr.is_empty());
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    assert_eq!(crate::run_with(["bogus".to_string()].into_iter(), &runtime, &mut stdout, &mut stderr), 2);
    assert!(stdout.is_empty() && String::from_utf8_lossy(&stderr).starts_with("usage: build-tracker init"));
    assert!(SystemClock.now() > FIXED_NOW);
    assert_eq!(object(vec![("a", Value::count(1))]), parse(r#"{"a":1}"#).expect("object"));
}

/// Files, issues and git held in memory, for the failures the real ones cannot be made to give.
struct Memory {
    files: HashMap<String, Result<String, FsError>>,
    failing_write: bool,
    issue_failure: Option<String>,
}

impl Files for Memory {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        self.files.get(path).cloned().unwrap_or_else(|| Err(FsError { code: "ENOENT".into(), message: format!("no {path}") }))
    }

    fn write_atomic(&self, path: &str, _content: &str) -> Result<(), FsError> {
        if self.failing_write {
            return Err(FsError { code: "EACCES".into(), message: format!("EACCES: permission denied, open '{path}'") });
        }
        Ok(())
    }

    fn list_files(&self, dir: &str) -> Result<Vec<String>, FsError> {
        let prefix = format!("{dir}/");
        Ok(self.files.keys().filter_map(|path| path.strip_prefix(&prefix).map(str::to_string)).collect())
    }
}

impl Issues for Memory {
    fn issue(&self, repository: &RepositoryName, number: u64) -> Result<IssueRecord, String> {
        match &self.issue_failure {
            Some(message) => Err(message.clone()),
            None => Ok(IssueRecord { closed: false, number, repository: repository.clone(), title: "Spec".into() }),
        }
    }

    fn sub_issues(&self, _repository: &RepositoryName, _number: u64) -> Result<Vec<IssueRecord>, String> {
        Ok(Vec::new())
    }

    fn blocked_by(&self, _repository: &RepositoryName, _number: u64) -> Result<Vec<IssueRecord>, String> {
        Ok(Vec::new())
    }
}

struct InRepository;

impl Repositories for InRepository {
    fn locate(&self, dir: &str) -> Result<Option<RepositoryLocation>, String> {
        Ok(Some(RepositoryLocation { main_root: "/r".into(), prefix: dir.strip_prefix("/r/").map(|rest| format!("{rest}/")).unwrap_or_default(), remote: None, root: "/r".into() }))
    }
}

fn memory_runtime(memory: Memory) -> Runtime {
    let shared = std::rc::Rc::new(memory);
    // The runtime owns boxed ports; each box shares the one memory.
    struct Shared(std::rc::Rc<Memory>);
    impl Files for Shared {
        fn read_text(&self, path: &str) -> Result<String, FsError> {
            self.0.read_text(path)
        }
        fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
            self.0.write_atomic(path, content)
        }
        fn list_files(&self, dir: &str) -> Result<Vec<String>, FsError> {
            self.0.list_files(dir)
        }
    }
    impl Issues for Shared {
        fn issue(&self, repository: &RepositoryName, number: u64) -> Result<IssueRecord, String> {
            self.0.issue(repository, number)
        }
        fn sub_issues(&self, repository: &RepositoryName, number: u64) -> Result<Vec<IssueRecord>, String> {
            self.0.sub_issues(repository, number)
        }
        fn blocked_by(&self, repository: &RepositoryName, number: u64) -> Result<Vec<IssueRecord>, String> {
            self.0.blocked_by(repository, number)
        }
    }
    Runtime {
        clock: Box::new(super::scenarios::FixedClock(FIXED_NOW)),
        cwd: "/r".into(),
        env: HashMap::from([("XDG_STATE_HOME".to_string(), "/state".to_string())]),
        files: Box::new(Shared(shared.clone())),
        issues: Box::new(Shared(shared)),
        repositories: Box::new(InRepository),
    }
}

fn memory(files: Vec<(&str, Result<&str, &str>)>) -> Memory {
    let files = files.into_iter().map(|(path, content)| (path.to_string(), content.map(str::to_string).map_err(|message| FsError { code: "EACCES".into(), message: message.to_string() }))).collect();
    Memory { failing_write: false, files, issue_failure: None }
}

#[test]
fn a_tracker_that_fails_without_a_message_is_still_named() {
    let mut tracker = memory(vec![]);
    tracker.issue_failure = Some(String::new());
    let result = crate::cli::run_cli(&["init".to_string(), "--spec".to_string(), "acme/widgets#10".to_string()], &memory_runtime(tracker));
    assert_eq!(result.stdout, "{\"code\":\"tracker-unreachable\",\"message\":\"cannot read the tickets of acme/widgets#10: the tracker failed\",\"status\":\"error\"}\n");
}

#[test]
fn a_ticket_file_that_cannot_be_read_stops_the_spec() {
    let files = vec![("/r/docs/spec.md", Ok("# Spec")), ("/r/docs/issues/01-a.md", Err("EACCES: permission denied, open '/r/docs/issues/01-a.md'"))];
    let result = crate::cli::run_cli(&["path".to_string(), "--spec".to_string(), "docs/spec.md".to_string()], &memory_runtime(memory(files.clone())));
    assert_eq!(result.exit_code, 0, "path reads no tickets");
    let result = crate::cli::run_cli(&["init".to_string(), "--spec".to_string(), "docs/spec.md".to_string()], &memory_runtime(memory(files)));
    assert_eq!(result.stderr, "build-tracker init: EACCES: permission denied, open '/r/docs/issues/01-a.md'\n");
}

#[test]
fn a_state_that_cannot_be_written_is_reported_and_the_page_is_not_left_behind() {
    let state = r#"{"version":1,"spec":"acme/widgets#10","title":"T","tickets":{"1":{"title":"a","blocked_by":[],"status":"todo"}}}"#;
    let mut files = memory(vec![("/state/state.json", Ok(state))]);
    files.failing_write = true;
    let runtime = memory_runtime(files);
    let set = crate::cli::run_cli(&["set".to_string(), "--state".to_string(), "/state/state.json".to_string(), "1=running".to_string()], &runtime);
    assert_eq!(set.stderr, "build-tracker set: EACCES: permission denied, open '/state/state.json'\n");
    let render = crate::cli::run_cli(&["render".to_string(), "--state".to_string(), "/state/state.json".to_string(), "--out".to_string(), "/out/p.html".to_string()], &runtime);
    assert_eq!(render.stderr, "build-tracker render: EACCES: permission denied, open '/out/p.html'\n");
}

#[test]
fn sync_gives_a_known_entry_without_a_status_the_todo_status() {
    use crate::seed::{sync_state, IssueTrackerSnapshot, SpecTicket};
    let raw = parse(r#"{"tickets":{"1":{"title":"x"}}}"#).expect("raw");
    let snapshot = IssueTrackerSnapshot { title: "T".into(), tickets: vec![SpecTicket { blocked_by: vec![], closed: false, external: vec![], file: None, number: 1, title: "x".into() }] };
    let outcome = sync_state(raw.as_object().expect("object"), &snapshot);
    assert_eq!(compact(&Value::Obj(outcome.raw)), r#"{"tickets":{"1":{"title":"x","blocked_by":[],"status":"todo"}}}"#);
}
