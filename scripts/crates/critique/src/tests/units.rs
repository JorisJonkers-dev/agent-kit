//! Each module's own behaviour, beside the replay of the goldens.

use super::cli::runtime;
use super::scenarios::Sandbox;
use crate::allow;
use crate::allowlist::{parse_allowlist, Allowlist, Entry};
use crate::error::{CritiqueError, ShapeError};
use crate::fields::Fields;
use crate::git::{failure_words, parse_remote_line, Git, SystemGit};
use crate::repository::{read_allowlist, resolve, Resolution};
use crate::Runtime;
use model_gateway::json::{parse, Value};
use model_gateway::ports::GitRemote;
use std::collections::HashMap;

fn json(text: &str) -> Value {
    parse(text).expect("json")
}

fn shape(text: &str, read: impl Fn(&Fields) -> Result<String, ShapeError>) -> String {
    let value = json(text);
    match Fields::of(&value, "").and_then(|fields| read(&fields)) {
        Ok(found) => found,
        Err(error) => error.0,
    }
}

#[test]
fn fields_name_the_path_of_the_wrong_field() {
    let string = |fields: &Fields| fields.string("a");
    assert_eq!(shape("{\"a\":\"x\"}", string), "x");
    assert_eq!(shape("{\"a\":\"  \"}", string), "a must be a non-empty string");
    assert_eq!(shape("{\"a\":5}", string), "a must be a non-empty string");
    assert_eq!(shape("{}", string), "a must be a non-empty string");
    assert_eq!(shape("[]", string), "the value must be a JSON object");
    assert_eq!(shape("{\"a\":{\"b\":5}}", |fields| fields.object("a")?.string("b")), "a.b must be a non-empty string");
    assert_eq!(shape("{\"a\":5}", |fields| fields.object("a").map(|_| String::new())), "a must be a JSON object");
    assert_eq!(shape("{}", |fields| fields.object("a").map(|_| String::new())), "a must be a JSON object");
    assert_eq!(shape("{\"a\":null}", |fields| Ok(fields.has("a").to_string())), "true");
    assert_eq!(shape("{}", |fields| Ok(fields.has("a").to_string())), "false");
    assert_eq!(shape("{\"a\":true}", |fields| fields.boolean("a").map(|flag| flag.to_string())), "true");
    assert_eq!(shape("{\"a\":\"true\"}", |fields| fields.boolean("a").map(|flag| flag.to_string())), "a must be true or false");
    assert_eq!(shape("{\"a\":1}", |fields| fields.literal("a", 1.0).map(|()| "ok".to_string())), "ok");
    assert_eq!(shape("{\"a\":2}", |fields| fields.literal("a", 1.0).map(|()| "ok".to_string())), "a must be 1");
    assert_eq!(shape("{\"a\":\"1\"}", |fields| fields.literal("a", 1.0).map(|()| "ok".to_string())), "a must be 1");
    let families = |fields: &Fields| fields.one_of_list("a", &["x", "y"]).map(|found| found.join("+"));
    assert_eq!(shape("{\"a\":[\"x\",\"y\"]}", families), "x+y");
    assert_eq!(shape("{\"a\":\"x\"}", families), "a must be a list");
    assert_eq!(shape("{\"a\":[\"x\",\"z\"]}", families), "a[1] must be one of x, y");
    assert_eq!(shape("{\"a\":[1]}", families), "a[0] must be one of x, y");
    let entries = |fields: &Fields| fields.entries("m", |entry, name| entry.string("s").map(|text| format!("{name}={text}"))).map(|found| found.into_iter().map(|(_, text)| text).collect::<Vec<_>>().join(","));
    assert_eq!(shape("{\"m\":{\"b\":{\"s\":\"1\"},\"2\":{\"s\":\"2\"}}}", entries), "2=2,b=1");
    assert_eq!(shape("{\"m\":{\"b\":5}}", entries), "m.b must be a JSON object");
    assert_eq!(shape("{\"m\":{\"b\":{}}}", entries), "m.b.s must be a non-empty string");
}

#[test]
fn an_error_is_worded_with_its_place_and_a_file_failure_is_unexpected() {
    let error = ShapeError("x must be a list".to_string()).into_error("invalid-input", "allow input");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("invalid-input", "allow input: x must be a list"));
    let fs = CritiqueError::from_fs(model_gateway::ports::FsError { code: "ENOENT".to_string(), message: "ENOENT: gone".to_string() });
    assert_eq!((fs.code.as_str(), fs.message.as_str()), ("unexpected", "ENOENT: gone"));
}

fn entry(families: &[&str], code: bool, history: bool, at: &str) -> Entry {
    Entry { families: families.iter().map(|family| (*family).to_string()).collect(), code, history, updated_at: at.to_string() }
}

#[test]
fn the_allowlist_reads_and_writes_the_file_the_node_runner_does() {
    let list = Allowlist::default().with_entry("b", entry(&["zhipu"], true, false, "t1")).with_entry("a", entry(&[], false, true, "t2")).with_entry("b", entry(&["google"], false, false, "t3"));
    let keys: Vec<&str> = list.repositories.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["b", "a"]);
    assert_eq!(list.get("b").map(|found| found.updated_at.as_str()), Some("t3"));
    assert!(list.get("c").is_none());
    let text = list.to_text();
    assert!(text.starts_with("{\n  \"version\": 1,\n  \"repositories\": {\n    \"b\": {\n      \"code\": false,\n      \"families\": [\n        \"google\"\n      ],"), "{text}");
    assert!(text.ends_with("  }\n}\n"), "{text}");
    assert_eq!(parse_allowlist(&text), Ok(list));
    let older = parse_allowlist("{\"version\":1,\"repositories\":{\"a\":{\"families\":[\"openai\"],\"updated_at\":\"x\"}}}").expect("parses");
    assert_eq!(older.get("a"), Some(&entry(&["openai"], false, false, "x")));
    for (text, message) in [
        ("{", "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
        ("[]", "the value must be a JSON object"),
        ("{\"version\":1}", "repositories must be a JSON object"),
        ("{\"version\":2,\"repositories\":{}}", "version must be 1"),
        ("{\"version\":1,\"repositories\":{\"a\":{\"families\":[],\"updated_at\":\"x\",\"code\":null}}}", "repositories.a.code must be true or false"),
        ("{\"version\":1,\"repositories\":{\"a\":{\"families\":[],\"updated_at\":\"x\",\"history\":null}}}", "repositories.a.history must be true or false"),
    ] {
        assert_eq!(parse_allowlist(text), Err(ShapeError(message.to_string())), "{text}");
    }
}

#[test]
fn a_remote_line_is_read_as_the_node_runner_reads_it() {
    let remote = |name: &str, url: &str| Some(GitRemote { name: name.to_string(), url: url.to_string() });
    assert_eq!(parse_remote_line("remote.origin.url git@github.com:a/b.git"), remote("origin", "git@github.com:a/b.git"));
    assert_eq!(parse_remote_line("remote.my.url.name.url https://x/y z"), remote("my.url.name", "https://x/y z"));
    assert_eq!(parse_remote_line("remote.a.url b.url c"), remote("a.url b", "c"));
    assert_eq!(parse_remote_line("remote.a.url  b"), remote("a", " b"));
    for line in ["origin.url x", "remote.origin.url", "remote.origin.url ", "remote..url x", "remote.origin.pushurl x", "remote.o\r.url x", "remote.o.url x\u{2028}y", ""] {
        assert_eq!(parse_remote_line(line), None, "{line:?}");
    }
}

#[test]
fn what_git_said_is_what_a_failure_reports() {
    assert_eq!(failure_words(&["rev-parse", "--x"], "/r", 128, "out", " err \n"), "git rev-parse --x failed in /r: err");
    assert_eq!(failure_words(&["a"], "/r", 1, " out ", ""), "git a failed in /r: out");
    assert_eq!(failure_words(&["a"], "/r", 7, "", " "), "git a failed in /r: exit code 7");
}

fn system_git(sandbox: &Sandbox, path: &str) -> SystemGit {
    let env: HashMap<String, String> = [("PATH".to_string(), path.to_string()), ("HOME".to_string(), sandbox.text())].into_iter().collect();
    SystemGit { env }
}

fn host_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

#[test]
fn the_system_git_reads_a_repository_and_words_its_failures_as_the_node_adapter_does() {
    let _held = super::scenarios::EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let sandbox = Sandbox::new();
    let git = system_git(&sandbox, &host_path());
    let repo = format!("{}/repo", sandbox.text());
    std::fs::create_dir_all(&repo).expect("dir");
    let init = std::process::Command::new("git").args(["init", "-q", &repo]).env("HOME", sandbox.text()).output().expect("git init");
    assert!(init.status.success());
    assert_eq!(git.remotes(&repo), Ok(vec![]));
    let added = std::process::Command::new("git").args(["-C", &repo, "remote", "add", "origin", "git@github.com:a/b.git"]).env("HOME", sandbox.text()).output().expect("git remote");
    assert!(added.status.success());
    assert_eq!(git.remotes(&repo), Ok(vec![GitRemote { name: "origin".to_string(), url: "git@github.com:a/b.git".to_string() }]));
    let real = std::fs::canonicalize(&repo).expect("canonical").to_string_lossy().into_owned();
    assert_eq!(git.root(&repo), Ok(real.clone()));
    assert_eq!(git.common_dir(&repo), Ok(format!("{real}/.git")));
    let plain = sandbox.text();
    let error = git.root(&plain).expect_err("not a repository");
    assert!(error.starts_with(&format!("git rev-parse --show-toplevel failed in {plain}: fatal: not a git repository")), "{error}");
    assert_eq!(git.root(&format!("{plain}/missing")), Err("spawn git ENOENT".to_string()));
    // A program named git that may not be run: the platform's own code.
    let bin = format!("{plain}/bin");
    std::fs::create_dir_all(&bin).expect("bin");
    std::fs::write(format!("{bin}/git"), "").expect("file");
    assert_eq!(system_git(&sandbox, &bin).root(&plain), Err("spawn git EACCES".to_string()));
}

struct Scripted {
    remotes: Result<Vec<GitRemote>, String>,
    common: Result<String, String>,
    root: Result<String, String>,
}

impl Git for Scripted {
    fn remotes(&self, _cwd: &str) -> Result<Vec<GitRemote>, String> {
        self.remotes.clone()
    }

    fn common_dir(&self, _cwd: &str) -> Result<String, String> {
        self.common.clone()
    }

    fn root(&self, _cwd: &str) -> Result<String, String> {
        self.root.clone()
    }

    fn resolve_commit(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn last_change(&self, _cwd: &str, _path: &str) -> Result<Option<crate::git::LastChange>, String> {
        Err("unused".to_string())
    }

    fn file_commits(&self, _cwd: &str, _revision: &str, _path: &str) -> Result<Vec<crate::git::LastChange>, String> {
        Err("unused".to_string())
    }

    fn file_at(&self, _cwd: &str, _commit: &str, _path: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn tree_files(&self, _cwd: &str, _commit: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn untracked_paths(&self, _cwd: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn merge_base(&self, _cwd: &str, _a: &str, _b: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn diff_commits(&self, _cwd: &str, _base: &str, _head: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn changed_paths(&self, _cwd: &str, _base: &str, _head: &str) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn staged_entries(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn tree_of(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn write_index_tree(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn commit_tree(&self, _cwd: &str, _tree: &str, _parent: &str, _message: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn commits_between(&self, _cwd: &str, _base: &str, _head: &str) -> Result<Vec<crate::git::CommitLink>, String> {
        Err("unused".to_string())
    }

    fn tree_entries(&self, _cwd: &str, _revision: &str) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn merge_tree(&self, _cwd: &str, _ours: &str, _theirs: &str) -> Result<crate::git::MergeTree, String> {
        Err("unused".to_string())
    }

    fn diff_against_head(&self, _cwd: &str, _staged: bool) -> Result<String, String> {
        Err("unused".to_string())
    }

    fn paths_against_head(&self, _cwd: &str, _staged: bool) -> Result<Vec<String>, String> {
        Err("unused".to_string())
    }

    fn current_branch(&self, _cwd: &str) -> Result<String, String> {
        Err("unused".to_string())
    }
}

fn origin() -> Vec<GitRemote> {
    vec![GitRemote { name: "origin".to_string(), url: "git@github.com:Owner/Repo.git".to_string() }]
}

fn runtime_with(git: Scripted, sandbox: &Sandbox) -> Runtime {
    let mut base = runtime("");
    base.git = Box::new(git);
    base.files = Box::new(model_gateway::adapters::fs::SystemFiles { cwd: sandbox.text() });
    base.env = [("HOME".to_string(), sandbox.text())].into_iter().collect();
    base
}

#[test]
fn a_repository_resolves_through_its_remotes_then_its_directories() {
    let sandbox = Sandbox::new();
    let good = Scripted { remotes: Ok(origin()), common: Ok("/r/.git".to_string()), root: Ok("/r".to_string()) };
    let Ok(Resolution::Resolved(found)) = resolve(&runtime_with(good, &sandbox), "/r") else {
        panic!("a repository with an origin resolves");
    };
    assert_eq!((found.key.as_str(), found.root.as_str(), found.common_dir.as_str()), ("github.com/owner/repo", "/r", "/r/.git"));
    let broken = |remotes: Result<Vec<GitRemote>, String>, common: Result<String, String>, root: Result<String, String>| resolve(&runtime_with(Scripted { remotes, common, root }, &sandbox), "/r").err();
    assert_eq!(broken(Err("boom".to_string()), Ok(String::new()), Ok(String::new())), Some(CritiqueError::unexpected("boom")));
    assert_eq!(broken(Ok(origin()), Err("no common".to_string()), Ok(String::new())), Some(CritiqueError::unexpected("no common")));
    assert_eq!(broken(Ok(origin()), Ok(String::new()), Err("no root".to_string())), Some(CritiqueError::unexpected("no root")));
}

#[test]
fn allow_reports_what_it_could_not_read_or_write() {
    let sandbox = Sandbox::new();
    let git = || Scripted { remotes: Ok(origin()), common: Ok("/r/.git".to_string()), root: Ok("/r".to_string()) };
    let input = json("{\"repo\":\"/r\",\"families\":[\"zhipu\"]}");
    let mut runtime = runtime_with(git(), &sandbox);
    runtime.env.clear();
    assert_eq!(allow::run(&input, &runtime), Err(CritiqueError::unexpected("required environment variable is missing: HOME")));
    assert_eq!(read_allowlist(&runtime).err(), Some(CritiqueError::unexpected("required environment variable is missing: HOME")));
    let runtime = runtime_with(git(), &sandbox);
    let file = format!("{}/.config/agent-kit/critique-allowlist.json", sandbox.text());
    std::fs::create_dir_all(&file).expect("a directory where the file would be");
    let error = read_allowlist(&runtime).expect_err("a directory is not a file");
    assert_eq!(error.code, "unexpected");
    let error = allow::run(&input, &runtime).expect_err("nor can it be written");
    assert_eq!(error.code, "unexpected");
    std::fs::remove_dir_all(&file).expect("clear");
    std::fs::create_dir_all(format!("{}/.config/agent-kit", sandbox.text())).expect("dir");
    std::fs::write(&file, "{").expect("file");
    assert_eq!(read_allowlist(&runtime).err().map(|error| error.code), Some("invalid-allowlist".to_string()));
    let refused = allow::run(&input, &runtime_with(Scripted { remotes: Ok(vec![]), common: Ok(String::new()), root: Ok(String::new()) }, &sandbox)).expect("a refusal is a result");
    assert_eq!(model_gateway::json::pretty(&refused), "{\n  \"message\": \"the repository has no remote, so it counts as not opted in\",\n  \"reason\": \"no-remote\",\n  \"status\": \"refused\"\n}");
}

#[test]
fn the_real_runtime_is_built_from_the_environment() {
    let runtime = crate::default_runtime();
    assert_eq!(runtime.env.get("PATH").cloned().unwrap_or_default(), host_path());
    assert!(runtime.clock.now_ms() > 1_700_000_000_000);
}

#[test]
fn the_output_goes_to_the_two_streams_and_the_exit_code_comes_back() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = crate::run_with(["bogus".to_string()].into_iter(), &runtime(""), &mut out, &mut err);
    assert_eq!((code, out.len(), String::from_utf8_lossy(&err).starts_with("usage: critique ")), (2, 0, true));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = crate::run_with(["-h".to_string()].into_iter(), &runtime(""), &mut out, &mut err);
    assert_eq!((code, err.len(), String::from_utf8_lossy(&out).starts_with("usage: critique ")), (0, 0, true));
}

/// The child of `standard_input_is_read_whole`: it reads the real standard input through the real runtime.
#[test]
fn standard_input_child() {
    if std::env::var("CRITIQUE_STDIN_CHILD").is_ok() {
        println!("stdin<<{}>>", (crate::default_runtime().stdin)());
    }
}

#[test]
fn standard_input_is_read_whole() {
    use std::io::Write;
    let mut child = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args(["--exact", "tests::units::standard_input_child", "--nocapture", "--test-threads=1"])
        .env("CRITIQUE_STDIN_CHILD", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("run the child");
    child.stdin.take().expect("stdin").write_all("h\u{e9}llo\n".as_bytes()).expect("write");
    let output = child.wait_with_output().expect("child ends");
    assert!(String::from_utf8_lossy(&output.stdout).contains("stdin<<h\u{e9}llo\n>>"), "{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn a_check_id_is_a_lower_case_slug() {
    use crate::checks::is_check_id;
    assert!(is_check_id(Some("no-rollback")));
    assert!(is_check_id(Some("a1")));
    assert!(!is_check_id(Some("No-Rollback")));
    assert!(!is_check_id(Some("a--b")));
    assert!(!is_check_id(None));
}
