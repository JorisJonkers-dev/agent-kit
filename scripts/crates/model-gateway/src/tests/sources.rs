//! Tests for the clients' own listings, the Vibe probe with its cache, and the
//! clients' own status commands.

use super::scenarios::{FixedClock, Sandbox, EXEC_LOCK};
use crate::adapters::fs::SystemFiles;
use crate::adapters::process::SystemProcess;
use crate::json::{self, Object, Value};
use crate::models::{ClientListing, ClientModel};
use crate::ports::{FileStamp, Files, FsError, Git, GitRemote, LockTimings, Process, ProcessCommand, ProcessResult, Runtime};
use crate::sources::list_client;
use crate::status::{read_status, STATUS_TIMEOUT_MS};
use crate::usage::{ClientStatusReading, SignedIn};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

/// 2026-11-03T12:00:00Z, the instant every test reads the clock at.
const NOW: i64 = 1_793_707_200_000;

const HOME: &str = "/home/test";
const CONFIG: &str = "/home/test/.vibe/config.toml";
const CACHE: &str = "/cache/model-gateway/vibe-available.json";

const AGY: &str = "agy models";
const OPENCODE: &str = "opencode models";
const HERMES: &str = "hermes config get model --json";
const PROBE: &str = "vibe --model model-gateway-probe --max-turns 1 --max-tokens 1 --max-price 0 -p ping";
const CODEX_STATUS: &str = "codex login status";
const HERMES_STATUS: &str = "hermes auth list";
const OPENCODE_STATUS: &str = "opencode auth list";

/// What the Vibe probe answers: Vibe refuses the probe id and names the aliases it accepts.
const VIBE_PROBE: &str =
    "Error: Unknown model: model-gateway-probe. Available: glm-5-3, mistral-large-4-0-3, mistral-medium-3.5, zai-glm-5-3-flash\n";

/// The aliases the probe above names, in the order it names them.
const VIBE_ALIASES: [&str; 4] = ["glm-5-3", "mistral-large-4-0-3", "mistral-medium-3.5", "zai-glm-5-3-flash"];

/// The files a fake file system holds, shared with the test so it can read what a call wrote.
#[derive(Clone, Default)]
struct Memory(Rc<RefCell<HashMap<String, String>>>);

impl Memory {
    fn get(&self, path: &str) -> Option<String> {
        self.0.borrow().get(path).cloned()
    }

    fn put(&self, path: &str, content: &str) {
        self.0.borrow_mut().insert(path.to_string(), content.to_string());
    }
}

/// A file system held in memory: the files a test writes, the stamps it gives
/// them, and the paths it refuses to look at, read or write.
struct FakeFiles {
    files: Memory,
    stamps: HashMap<String, FileStamp>,
    denied: Vec<String>,
    unreadable: Vec<String>,
    unwritable: Vec<String>,
}

impl FakeFiles {
    fn new() -> FakeFiles {
        FakeFiles {
            files: Memory::default(),
            stamps: HashMap::new(),
            denied: Vec::new(),
            unreadable: Vec::new(),
            unwritable: Vec::new(),
        }
    }

    fn with(self, path: &str, content: &str) -> FakeFiles {
        self.files.put(path, content);
        self
    }

    fn stamped(mut self, path: &str, stamp: FileStamp) -> FakeFiles {
        self.stamps.insert(path.to_string(), stamp);
        self
    }

    fn denying(mut self, path: &str) -> FakeFiles {
        self.denied.push(path.to_string());
        self
    }

    fn unreadable(mut self, path: &str) -> FakeFiles {
        self.unreadable.push(path.to_string());
        self
    }

    fn unwritable(mut self, path: &str) -> FakeFiles {
        self.unwritable.push(path.to_string());
        self
    }

    fn memory(&self) -> Memory {
        self.files.clone()
    }
}

fn failure(path: &str) -> FsError {
    FsError { code: "EACCES".to_string(), message: format!("EACCES: permission denied, access '{path}'") }
}

impl Files for FakeFiles {
    fn write_text(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        unreachable!("the test does not write text")
    }

    fn temp_dir(&self, _parent: &str, _prefix: &str) -> Result<String, FsError> {
        unreachable!("the test makes no temporary directory")
    }

    fn remove(&self, _path: &str) -> Result<(), FsError> {
        unreachable!("the test removes nothing")
    }

    fn read_text(&self, path: &str) -> Result<String, FsError> {
        if self.unreadable.iter().any(|unreadable| unreadable == path) {
            return Err(failure(path));
        }
        match self.files.get(path) {
            Some(content) => Ok(content),
            None => Err(FsError {
                code: "ENOENT".to_string(),
                message: format!("ENOENT: no such file or directory, open '{path}'"),
            }),
        }
    }

    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.files.put(path, content);
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn create_exclusive(&self, _path: &str) -> Result<bool, FsError> {
        Ok(true)
    }

    fn remove_file(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn modified_ms(&self, _path: &str) -> Option<i64> {
        None
    }

    fn exists(&self, path: &str) -> Result<bool, FsError> {
        if self.denied.iter().any(|denied| denied == path) {
            return Err(failure(path));
        }
        Ok(self.files.get(path).is_some())
    }

    fn append(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn stamp(&self, path: &str) -> Option<FileStamp> {
        self.stamps.get(path).cloned()
    }

    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        if self.unwritable.iter().any(|unwritable| unwritable == path) {
            return Err(failure(path));
        }
        self.files.put(path, content);
        Ok(())
    }
}

/// A program that answers the commands a test scripts, keyed by `command args`.
struct FakeProcess {
    resolved: HashMap<String, String>,
    answers: HashMap<String, Result<ProcessResult, String>>,
    ran: Rc<RefCell<Vec<String>>>,
    timeouts: Rc<RefCell<Vec<Option<u64>>>>,
}

impl FakeProcess {
    fn new() -> FakeProcess {
        FakeProcess {
            resolved: HashMap::new(),
            answers: HashMap::new(),
            ran: Rc::new(RefCell::new(Vec::new())),
            timeouts: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn answering(mut self, command: &str, result: ProcessResult) -> FakeProcess {
        self.answers.insert(command.to_string(), Ok(result));
        self
    }

    fn resolving(mut self, binary: &str, path: &str) -> FakeProcess {
        self.resolved.insert(binary.to_string(), path.to_string());
        self
    }
}

impl Process for FakeProcess {
    fn resolve(&self, binary: &str) -> Option<String> {
        self.resolved.get(binary).cloned()
    }

    fn exec(&self, command: &str, args: &[&str], timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        let key = std::iter::once(command).chain(args.iter().copied()).collect::<Vec<_>>().join(" ");
        self.ran.borrow_mut().push(key.clone());
        self.timeouts.borrow_mut().push(timeout_ms);
        match self.answers.get(&key) {
            Some(answer) => answer.clone(),
            None => Err(format!("cannot run {command}")),
        }
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        self.exec(&command.command, &args, command.timeout_ms)
    }
}

/// A git the listings never ask anything.
struct NoGit;

impl Git for NoGit {
    fn common_dir(&self, _path: &str) -> Result<String, String> {
        unreachable!("the test asks for no common directory")
    }

    fn add_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        unreachable!("the test adds no worktree")
    }

    fn root(&self, _path: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        Ok(Vec::new())
    }

    fn remove_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete_branch(&self, _repo: &str, _branch: &str) -> Result<(), String> {
        Ok(())
    }
}

fn env() -> HashMap<String, String> {
    HashMap::from([("HOME".to_string(), HOME.to_string()), ("XDG_CACHE_HOME".to_string(), "/cache".to_string())])
}

fn runtime(process: FakeProcess, files: FakeFiles, env: HashMap<String, String>) -> Runtime {
    Runtime {
        clock: Box::new(FixedClock(NOW)),
        files: Box::new(files),
        env,
        lock: LockTimings::default(),
        process: Box::new(process),
        git: Box::new(NoGit),
        defaults: String::new(),
        defaults_path: "defaults.json".to_string(),
        routing: crate::SHIPPED_ROUTING.to_string(),
    }
}

fn ok(stdout: &str) -> ProcessResult {
    ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out: false }
}

fn failed(exit_code: i32, stderr: &str) -> ProcessResult {
    ProcessResult { stdout: String::new(), stderr: stderr.to_string(), exit_code, timed_out: false }
}

fn probe(stderr: &str) -> ProcessResult {
    ProcessResult { stdout: String::new(), stderr: stderr.to_string(), exit_code: 1, timed_out: false }
}

fn stamp(realpath: &str, ctime_ms: f64, size: f64, ino: f64, sha256: &str) -> FileStamp {
    FileStamp { realpath: realpath.to_string(), ctime_ms, ino, sha256: sha256.to_string(), size }
}

fn model(model: &str) -> ClientModel {
    ClientModel { model: model.to_string(), names: Vec::new(), provider: None }
}

fn named(model: &str, names: &[&str]) -> ClientModel {
    ClientModel {
        model: model.to_string(),
        names: names.iter().map(|name| name.to_string()).collect(),
        provider: None,
    }
}

/// The aliases the probe names, as the gateway lists them with no configuration to join them by.
fn probe_listing() -> ClientListing {
    ClientListing::Models(VIBE_ALIASES.iter().map(|alias| model(alias)).collect())
}

/// A cache entry as the gateway writes it, with the members a test overrides.
fn entry(over: &[(&str, Value)]) -> String {
    let mut object = Object::new();
    object.set("binary", Value::Null);
    object.set("binary_ctime_ms", Value::Null);
    object.set("binary_ino", Value::Null);
    object.set("binary_size", Value::Null);
    object.set("config_path", Value::str(CONFIG));
    object.set("config_sha256", Value::Null);
    object.set("models", Value::Arr(vec![Value::str("glm-5-3")]));
    object.set("version", Value::Num(2.0));
    object.set("written_at_ms", Value::Num((NOW - 1_000) as f64));
    for (key, value) in over {
        object.set(key, value.clone());
    }
    json::compact(&Value::Obj(object))
}

/// The entry the probe's answer is kept in, byte for byte, with no Vibe binary and no configuration to key it by.
fn fresh_entry() -> String {
    format!(
        "{{\"binary\":null,\"binary_ctime_ms\":null,\"binary_ino\":null,\"binary_size\":null,\"config_path\":\"{CONFIG}\",\"config_sha256\":null,\"models\":[\"glm-5-3\",\"mistral-large-4-0-3\",\"mistral-medium-3.5\",\"zai-glm-5-3-flash\"],\"version\":2,\"written_at_ms\":{NOW}}}"
    )
}

#[test]
fn the_gateway_has_no_source_for_a_client_that_answers_for_nobody() {
    let runtime = runtime(FakeProcess::new(), FakeFiles::new(), env());

    assert_eq!(list_client(&runtime, "claude"), None);
    assert_eq!(list_client(&runtime, "codex"), None);
}

#[test]
fn antigravity_lists_the_ids_its_own_listing_names() {
    let process = FakeProcess::new().answering(
        AGY,
        ok("Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\ngemini-4-pro\tGemini 4 Pro\n"),
    );
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "agy"),
        Some(ClientListing::Models(vec![model("gemini-3.8-flash-high"), model("gemini-4-pro")]))
    );
}

#[test]
fn opencode_lists_each_provider_model_its_own_listing_names() {
    let process = FakeProcess::new().answering(OPENCODE, ok("opencode-go/gemini-4-argon\nother/gemini-4-argon\nnot a model\n"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "opencode"),
        Some(ClientListing::Models(vec![
            ClientModel {
                model: "opencode-go/gemini-4-argon".to_string(),
                names: Vec::new(),
                provider: Some("opencode-go".to_string()),
            },
            ClientModel {
                model: "other/gemini-4-argon".to_string(),
                names: Vec::new(),
                provider: Some("other".to_string()),
            },
        ]))
    );
}

#[test]
fn hermes_lists_the_model_its_configuration_names() {
    let process = FakeProcess::new().answering(HERMES, ok("{\"default\":\"glm-5-3\",\"provider\":\"zhipu\"}"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "hermes"),
        Some(ClientListing::Models(vec![ClientModel {
            model: "glm-5-3".to_string(),
            names: Vec::new(),
            provider: Some("zhipu".to_string()),
        }]))
    );
}

#[test]
fn hermes_names_no_model_when_its_configuration_names_none() {
    let process = FakeProcess::new().answering(HERMES, ok("{}"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "hermes"),
        Some(ClientListing::Reason(format!("listing failed: {HERMES} named no model")))
    );
}

#[test]
fn a_listing_that_fails_names_the_command_its_exit_and_its_first_line_of_complaint() {
    let process = FakeProcess::new()
        .answering(AGY, failed(1, "not signed in\nsecond line\n"))
        .answering(OPENCODE, failed(2, ""))
        .answering(HERMES, failed(3, "  \n"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "agy"),
        Some(ClientListing::Reason("listing failed: agy models exited 1: not signed in".to_string()))
    );
    assert_eq!(
        list_client(&runtime, "opencode"),
        Some(ClientListing::Reason("listing failed: opencode models exited 2".to_string()))
    );
    assert_eq!(
        list_client(&runtime, "hermes"),
        Some(ClientListing::Reason("listing failed: hermes config get model --json exited 3".to_string()))
    );
}

#[test]
fn a_listing_that_cannot_be_run_names_the_command_and_why() {
    let runtime = runtime(FakeProcess::new(), FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "agy"),
        Some(ClientListing::Reason("listing failed: agy models: cannot run agy".to_string()))
    );
    assert_eq!(
        list_client(&runtime, "opencode"),
        Some(ClientListing::Reason("listing failed: opencode models: cannot run opencode".to_string()))
    );
    assert_eq!(
        list_client(&runtime, "hermes"),
        Some(ClientListing::Reason("listing failed: hermes config get model --json: cannot run hermes".to_string()))
    );
    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Reason(format!("listing failed: {PROBE}: cannot run vibe")))
    );
}

#[test]
fn vibe_lists_the_aliases_its_refusal_names_and_keeps_the_answer_for_the_calls_that_follow() {
    let files = FakeFiles::new();
    let memory = files.memory();
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let ran = process.ran.clone();
    let timeouts = process.timeouts.clone();
    let runtime = runtime(process, files, env());

    assert_eq!(list_client(&runtime, "vibe"), Some(probe_listing()));
    assert_eq!(*ran.borrow(), vec![PROBE.to_string()]);
    assert_eq!(*timeouts.borrow(), vec![Some(30_000)]);
    assert_eq!(memory.get(CACHE), Some(fresh_entry()));

    // The answer is kept, so the call that follows reads it instead of starting Vibe again.
    assert_eq!(list_client(&runtime, "vibe"), Some(probe_listing()));
    assert_eq!(ran.borrow().len(), 1);
}

#[test]
fn vibe_joins_each_accepted_alias_to_the_name_its_configuration_gives_it() {
    let files = FakeFiles::new()
        .with(
            CONFIG,
            "[[models]]\nname = \"mistral-large-latest\"\nprovider = \"mistral\"\nalias = \"mistral-large-4-0-3\"\n",
        )
        .stamped(CONFIG, stamp(CONFIG, 1_700_000_000_000.0, 96.0, 7.0, "cfg"))
        .stamped("/usr/bin/vibe", stamp("/opt/vibe/bin/vibe", 1_700_000_000_001.0, 1024.0, 42.0, "bin"));
    let memory = files.memory();
    let process = FakeProcess::new().resolving("vibe", "/usr/bin/vibe").answering(PROBE, probe(VIBE_PROBE));
    let runtime = runtime(process, files, env());

    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Models(vec![
            model("glm-5-3"),
            named("mistral-large-4-0-3", &["mistral-large-latest"]),
            model("mistral-medium-3.5"),
            model("zai-glm-5-3-flash"),
        ]))
    );
    assert_eq!(
        memory.get(CACHE),
        Some(format!(
            "{{\"binary\":\"/opt/vibe/bin/vibe\",\"binary_ctime_ms\":1700000000001,\"binary_ino\":42,\"binary_size\":1024,\"config_path\":\"{CONFIG}\",\"config_sha256\":\"cfg\",\"models\":[\"glm-5-3\",\"mistral-large-4-0-3\",\"mistral-medium-3.5\",\"zai-glm-5-3-flash\"],\"version\":2,\"written_at_ms\":{NOW}}}"
        ))
    );
}

#[test]
fn vibe_reads_the_configuration_from_vibe_home_when_it_is_set() {
    let files = FakeFiles::new().with(
        "/vibe-home/config.toml",
        "[[models]]\nname = \"mistral-large-latest\"\nprovider = \"mistral\"\nalias = \"mistral-large-4-0-3\"\n",
    );
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let mut env = env();
    env.insert("VIBE_HOME".to_string(), "/vibe-home".to_string());
    let runtime = runtime(process, files, env);

    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Models(vec![
            model("glm-5-3"),
            named("mistral-large-4-0-3", &["mistral-large-latest"]),
            model("mistral-medium-3.5"),
            model("zai-glm-5-3-flash"),
        ]))
    );
}

#[test]
fn vibe_reads_the_aliases_from_the_stdout_as_well_as_the_stderr_it_may_answer_on() {
    let process = FakeProcess::new().answering(
        PROBE,
        ProcessResult {
            stdout: "Error: Unknown model: model-gateway-probe. Available: glm-5-3, mistral-medium-3.5\n".to_string(),
            stderr: String::new(),
            exit_code: 1,
            timed_out: false,
        },
    );
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Models(vec![model("glm-5-3"), model("mistral-medium-3.5")]))
    );
}

#[test]
fn vibe_tolerates_the_warning_lines_it_prints_before_its_refusal() {
    let process = FakeProcess::new().answering(
        PROBE,
        probe("Warning: MCP server memory failed to connect\nWarning: MCP server fetch failed to connect\nError: Unknown model: model-gateway-probe. Available: glm-5-3, mistral-medium-3.5\n"),
    );
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Models(vec![model("glm-5-3"), model("mistral-medium-3.5")]))
    );
}

#[test]
fn vibe_offers_no_model_and_names_the_reason_when_the_probe_answers_nothing_it_can_read() {
    let process = FakeProcess::new().answering(PROBE, probe("Error: something else\n"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Reason(format!("listing failed: {PROBE} exited 1: Error: something else")))
    );
}

#[test]
fn vibe_offers_no_model_and_names_the_reason_when_the_probe_does_not_answer_in_time() {
    let process = FakeProcess::new().answering(
        PROBE,
        ProcessResult { stdout: String::new(), stderr: String::new(), exit_code: 124, timed_out: true },
    );
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Reason(format!("listing failed: {PROBE} did not answer within 30s")))
    );
}

#[test]
fn vibe_offers_no_model_and_runs_no_probe_when_the_configuration_declares_the_probe_id_as_a_model() {
    let files = FakeFiles::new()
        .with(CONFIG, "[[models]]\nname = \"glm-5-3\"\nprovider = \"mistral\"\nalias = \"model-gateway-probe\"\n")
        .with(CACHE, &entry(&[]));
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let ran = process.ran.clone();
    let runtime = runtime(process, files, env());

    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Reason(format!(
            "listing failed: {CONFIG} declares the gateway's probe id 'model-gateway-probe' as a model, so the probe could run it instead of being refused: rename or remove the alias"
        )))
    );
    assert!(ran.borrow().is_empty());
}

#[test]
fn vibe_joins_no_name_when_the_configuration_is_not_toml_and_still_lists_every_alias_it_accepts() {
    let files = FakeFiles::new().with(CONFIG, "this is not TOML\n");
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let runtime = runtime(process, files, env());

    assert_eq!(list_client(&runtime, "vibe"), Some(probe_listing()));
}

#[test]
fn vibe_joins_no_name_when_the_configuration_cannot_be_read_or_looked_at() {
    let unreadable = FakeFiles::new().with(CONFIG, "[[models]]\nname = \"glm-5-3\"\nalias = \"glm-5-3\"\n").unreadable(CONFIG);
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    assert_eq!(list_client(&runtime(process, unreadable, env()), "vibe"), Some(probe_listing()));

    let denied = FakeFiles::new().denying(CONFIG);
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    assert_eq!(list_client(&runtime(process, denied, env()), "vibe"), Some(probe_listing()));
}

#[test]
fn vibe_lists_anyway_keeping_nothing_on_a_machine_whose_environment_names_no_home() {
    let files = FakeFiles::new();
    let memory = files.memory();
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let runtime = runtime(process, files, HashMap::new());

    assert_eq!(list_client(&runtime, "vibe"), Some(probe_listing()));
    assert_eq!(memory.get(CACHE), None);
}

#[test]
fn vibe_lists_anyway_when_the_answer_cannot_be_kept() {
    let files = FakeFiles::new().unwritable(CACHE);
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let runtime = runtime(process, files, env());

    assert_eq!(list_client(&runtime, "vibe"), Some(probe_listing()));
}

#[test]
fn an_answer_this_vibe_gave_inside_its_window_is_read_without_probing() {
    let files = FakeFiles::new().with(CACHE, &entry(&[]));
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let ran = process.ran.clone();
    let runtime = runtime(process, files, env());

    assert_eq!(list_client(&runtime, "vibe"), Some(ClientListing::Models(vec![model("glm-5-3")])));
    assert!(ran.borrow().is_empty());
}

#[test]
fn an_answer_is_read_only_for_the_vibe_and_the_configuration_that_gave_it() {
    let key = [
        ("binary", Value::str("/opt/vibe/bin/vibe")),
        ("binary_ctime_ms", Value::Num(1_700_000_000_001.0)),
        ("binary_ino", Value::Num(42.0)),
        ("binary_size", Value::Num(1024.0)),
        ("config_sha256", Value::str("cfg")),
        ("models", Value::Arr(vec![Value::str("glm-5-3"), Value::str("mistral-large-4-0-3")])),
    ];
    let files = FakeFiles::new()
        .with(CACHE, &entry(&key))
        .with(
            CONFIG,
            "[[models]]\nname = \"mistral-large-latest\"\nprovider = \"mistral\"\nalias = \"mistral-large-4-0-3\"\n",
        )
        .stamped(CONFIG, stamp(CONFIG, 1_700_000_000_000.0, 96.0, 7.0, "cfg"))
        .stamped("/usr/bin/vibe", stamp("/opt/vibe/bin/vibe", 1_700_000_000_001.0, 1024.0, 42.0, "bin"));
    let memory = files.memory();
    let process = FakeProcess::new().resolving("vibe", "/usr/bin/vibe").answering(PROBE, probe(VIBE_PROBE));
    let ran = process.ran.clone();
    let runtime = runtime(process, files, env());

    // The answer is read, and joined to the names the configuration gives it, without probing.
    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Models(vec![model("glm-5-3"), named("mistral-large-4-0-3", &["mistral-large-latest"])]))
    );
    assert!(ran.borrow().is_empty());

    // The same answer, keyed by no Vibe and no configuration, is not this Vibe's.
    memory.put(CACHE, &entry(&[]));
    assert_eq!(
        list_client(&runtime, "vibe"),
        Some(ClientListing::Models(vec![
            model("glm-5-3"),
            named("mistral-large-4-0-3", &["mistral-large-latest"]),
            model("mistral-medium-3.5"),
            model("zai-glm-5-3-flash"),
        ]))
    );
    assert_eq!(ran.borrow().len(), 1);
}

#[test]
fn an_answer_that_is_not_this_vibes_this_configurations_or_inside_its_window_is_no_answer() {
    let files = FakeFiles::new();
    let memory = files.memory();
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let ran = process.ran.clone();
    let runtime = runtime(process, files, env());
    let infinite = entry(&[]).replace(&format!("\"written_at_ms\":{}", NOW - 1_000), "\"written_at_ms\":1e999");
    let not_the_answer = [
        "null".to_string(),
        entry(&[("version", Value::Num(3.0))]),
        entry(&[("version", Value::str("2"))]),
        entry(&[("binary", Value::str("/opt/vibe/bin/vibe"))]),
        entry(&[("binary_ctime_ms", Value::Num(1.0))]),
        entry(&[("binary_size", Value::Num(1.0))]),
        entry(&[("binary_ino", Value::Num(1.0))]),
        entry(&[("config_path", Value::str("/home/test/other-vibe/config.toml"))]),
        entry(&[("config_sha256", Value::str("ef537f25c8959fa9b5defbbecb6a0dbf6d0e107ec3c2ecc9fa08a90ffd1e0036"))]),
        entry(&[("written_at_ms", Value::str("not a number"))]),
        entry(&[("written_at_ms", Value::Num((NOW - 600_000) as f64))]),
        entry(&[("written_at_ms", Value::Num((NOW + 60_000) as f64))]),
        entry(&[("models", Value::Arr(Vec::new()))]),
        entry(&[("models", Value::str("glm-5-3"))]),
        entry(&[("models", Value::Arr(vec![Value::Num(3.0)]))]),
        entry(&[("models", Value::Arr(vec![Value::str("--not an id")]))]),
        infinite,
    ];

    for (index, bad) in not_the_answer.iter().enumerate() {
        memory.put(CACHE, bad);
        assert_eq!(list_client(&runtime, "vibe"), Some(probe_listing()), "{bad}");
        assert_eq!(ran.borrow().len(), index + 1, "{bad}");
    }
    assert_eq!(memory.get(CACHE), Some(fresh_entry()));
}

#[test]
fn an_answer_file_that_is_not_json_or_cannot_be_read_is_no_answer() {
    let not_json = FakeFiles::new().with(CACHE, "not json");
    let memory = not_json.memory();
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let listing = runtime(process, not_json, env());
    assert_eq!(list_client(&listing, "vibe"), Some(probe_listing()));
    assert_eq!(memory.get(CACHE), Some(fresh_entry()));

    let unreadable = FakeFiles::new().with(CACHE, &entry(&[])).unreadable(CACHE);
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let ran = process.ran.clone();
    assert_eq!(list_client(&runtime(process, unreadable, env()), "vibe"), Some(probe_listing()));
    assert_eq!(ran.borrow().len(), 1);

    let denied = FakeFiles::new().denying(CACHE);
    let process = FakeProcess::new().answering(PROBE, probe(VIBE_PROBE));
    let ran = process.ran.clone();
    assert_eq!(list_client(&runtime(process, denied, env()), "vibe"), Some(probe_listing()));
    assert_eq!(ran.borrow().len(), 1);
}

#[test]
fn a_status_command_is_given_the_time_limit_it_is_handed() {
    let process = FakeProcess::new().answering(CODEX_STATUS, ok("Logged in using ChatGPT\n"));
    let timeouts = process.timeouts.clone();
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        read_status(&runtime, "codex", 50),
        Some(ClientStatusReading {
            command: "codex login status".to_string(),
            signed_in: SignedIn::Yes,
            detail: "Logged in using ChatGPT".to_string(),
        })
    );
    assert_eq!(*timeouts.borrow(), vec![Some(50)]);
}

#[test]
fn a_login_status_that_says_it_is_not_logged_in_is_signed_out() {
    let process = FakeProcess::new().answering(CODEX_STATUS, failed(1, "Not logged in\n"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        read_status(&runtime, "codex", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "codex login status".to_string(),
            signed_in: SignedIn::No,
            detail: "Not logged in".to_string(),
        })
    );
}

#[test]
fn a_login_status_that_failed_for_another_reason_is_unknown_not_signed_out() {
    let process = FakeProcess::new().answering(CODEX_STATUS, failed(2, "error: unrecognized subcommand 'login'\n"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        read_status(&runtime, "codex", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "codex login status".to_string(),
            signed_in: SignedIn::Unknown,
            detail: "could not be read: exited 2: error: unrecognized subcommand 'login'".to_string(),
        })
    );
}

#[test]
fn a_login_status_that_says_nothing_still_reads_its_exit() {
    let signed_in = FakeProcess::new().answering(CODEX_STATUS, ok(""));
    assert_eq!(
        read_status(&runtime(signed_in, FakeFiles::new(), env()), "codex", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "codex login status".to_string(),
            signed_in: SignedIn::Yes,
            detail: "exited 0".to_string(),
        })
    );

    let silent = FakeProcess::new().answering(CODEX_STATUS, failed(1, ""));
    assert_eq!(
        read_status(&runtime(silent, FakeFiles::new(), env()), "codex", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "codex login status".to_string(),
            signed_in: SignedIn::Unknown,
            detail: "could not be read: exited 1".to_string(),
        })
    );
}

#[test]
fn a_login_status_reads_the_stderr_when_the_stdout_says_nothing() {
    let process = FakeProcess::new().answering(CODEX_STATUS, failed(1, "  Not logged in  \n"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        read_status(&runtime, "codex", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "codex login status".to_string(),
            signed_in: SignedIn::No,
            detail: "Not logged in".to_string(),
        })
    );
}

#[test]
fn every_phrase_that_says_a_client_is_not_logged_in_reads_as_signed_out() {
    for said in ["logged out", "no credentials", "login required", "log in required", "unauthenticated", "UNAUTHENTICATED"] {
        let process = FakeProcess::new().answering(CODEX_STATUS, failed(1, &format!("{said}\n")));
        let reading = read_status(&runtime(process, FakeFiles::new(), env()), "codex", STATUS_TIMEOUT_MS);
        assert_eq!(reading.map(|reading| reading.signed_in), Some(SignedIn::No), "{said}");
    }
}

#[test]
fn a_status_command_that_does_not_answer_in_time_is_unknown() {
    let process = FakeProcess::new().answering(
        CODEX_STATUS,
        ProcessResult { stdout: String::new(), stderr: String::new(), exit_code: 124, timed_out: true },
    );
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        read_status(&runtime, "codex", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "codex login status".to_string(),
            signed_in: SignedIn::Unknown,
            detail: "status command did not answer in time".to_string(),
        })
    );
}

#[test]
fn a_status_command_that_cannot_be_run_is_unknown() {
    let runtime = runtime(FakeProcess::new(), FakeFiles::new(), env());

    assert_eq!(
        read_status(&runtime, "codex", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "codex login status".to_string(),
            signed_in: SignedIn::Unknown,
            detail: "could not be read: cannot run codex".to_string(),
        })
    );
}

#[test]
fn a_credential_listing_that_names_credentials_is_signed_in() {
    let process = FakeProcess::new()
        .answering(HERMES_STATUS, ok("copilot (1 credentials):\n  #1  gh auth token\n"))
        .answering(OPENCODE_STATUS, ok("OpenCode Console  Personal  stored\n"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        read_status(&runtime, "hermes", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "hermes auth list".to_string(),
            signed_in: SignedIn::Yes,
            detail: "it lists credentials".to_string(),
        })
    );
    assert_eq!(
        read_status(&runtime, "opencode", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "opencode auth list".to_string(),
            signed_in: SignedIn::Yes,
            detail: "it lists credentials".to_string(),
        })
    );
}

#[test]
fn a_credential_listing_that_names_none_is_signed_out() {
    let process = FakeProcess::new().answering(HERMES_STATUS, ok("\n"));
    let runtime = runtime(process, FakeFiles::new(), env());

    assert_eq!(
        read_status(&runtime, "hermes", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "hermes auth list".to_string(),
            signed_in: SignedIn::No,
            detail: "it lists no credentials".to_string(),
        })
    );
}

#[test]
fn a_credential_listing_that_failed_is_unknown_not_signed_out() {
    let complained = FakeProcess::new().answering(OPENCODE_STATUS, failed(2, "unknown command\n"));
    assert_eq!(
        read_status(&runtime(complained, FakeFiles::new(), env()), "opencode", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "opencode auth list".to_string(),
            signed_in: SignedIn::Unknown,
            detail: "could not be read: exited 2: unknown command".to_string(),
        })
    );

    let silent = FakeProcess::new().answering(OPENCODE_STATUS, failed(2, ""));
    assert_eq!(
        read_status(&runtime(silent, FakeFiles::new(), env()), "opencode", STATUS_TIMEOUT_MS),
        Some(ClientStatusReading {
            command: "opencode auth list".to_string(),
            signed_in: SignedIn::Unknown,
            detail: "could not be read: exited 2".to_string(),
        })
    );
}

#[test]
fn a_client_with_no_status_command_has_no_reading() {
    let runtime = runtime(FakeProcess::new(), FakeFiles::new(), env());

    assert_eq!(read_status(&runtime, "vibe", STATUS_TIMEOUT_MS), None);
    assert_eq!(read_status(&runtime, "agy", STATUS_TIMEOUT_MS), None);
    assert_eq!(STATUS_TIMEOUT_MS, 5_000);
}

/// A shell script in `directory`, executable.
fn script(directory: &Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    std::fs::write(&path, body).expect("write the script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("make the script executable");
    path
}

/// The real programs, run through the real process adapter, answer the listings the gateway reads.
#[test]
fn the_real_programs_answer_the_listings_the_gateway_reads() {
    let sandbox = Sandbox::new();
    let _guard = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    script(&sandbox.path, "agy", "#!/bin/sh\nprintf 'Fetching available models...\\ngemini-4-pro\\tGemini 4 Pro\\n'\n");
    script(&sandbox.path, "opencode", "#!/bin/sh\nprintf 'opencode-go/glm-5.3\\n'\n");
    script(&sandbox.path, "hermes", "#!/bin/sh\nprintf '{\"default\":\"glm-5-3\",\"provider\":\"zhipu\"}'\n");
    let mut env = HashMap::new();
    env.insert("PATH".to_string(), sandbox.text());
    env.insert("HOME".to_string(), sandbox.path.join("home").to_string_lossy().into_owned());
    let runtime = Runtime {
        clock: Box::new(FixedClock(NOW)),
        files: Box::new(SystemFiles { cwd: sandbox.text() }),
        lock: LockTimings::default(),
        process: Box::new(SystemProcess { env: env.clone() }),
        git: Box::new(NoGit),
        env,
        defaults: String::new(),
        defaults_path: "defaults.json".to_string(),
        routing: crate::SHIPPED_ROUTING.to_string(),
    };

    assert_eq!(list_client(&runtime, "agy"), Some(ClientListing::Models(vec![model("gemini-4-pro")])));
    assert_eq!(
        list_client(&runtime, "opencode"),
        Some(ClientListing::Models(vec![ClientModel {
            model: "opencode-go/glm-5.3".to_string(),
            names: Vec::new(),
            provider: Some("opencode-go".to_string()),
        }]))
    );
    assert_eq!(
        list_client(&runtime, "hermes"),
        Some(ClientListing::Models(vec![ClientModel {
            model: "glm-5-3".to_string(),
            names: Vec::new(),
            provider: Some("zhipu".to_string()),
        }]))
    );
}

/// The real Vibe probe, run through the real process adapter, keeps its answer where only its owner can read it.
#[test]
fn the_real_vibe_probe_keeps_its_answer_where_only_its_owner_can_read_it() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let _guard = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let marker = sandbox.path.join("probes");
    let body = format!(
        "#!/bin/sh\necho ran >> '{}'\nprintf 'Error: Unknown model: model-gateway-probe. Available: glm-5-3, mistral-medium-3.5\\n' >&2\nexit 1\n",
        marker.to_string_lossy()
    );
    script(&sandbox.path, "vibe", &body);
    let home = sandbox.path.join("home");
    std::fs::create_dir_all(home.join(".vibe")).expect("vibe home");
    std::fs::write(home.join(".vibe").join("config.toml"), "[[models]]\nname = \"glm-5-3-latest\"\nprovider = \"zhipu\"\nalias = \"glm-5-3\"\n")
        .expect("vibe config");
    let cache_home = sandbox.path.join("cache");
    let mut env = HashMap::new();
    env.insert("PATH".to_string(), sandbox.text());
    env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    env.insert("XDG_CACHE_HOME".to_string(), cache_home.to_string_lossy().into_owned());
    let runtime = Runtime {
        clock: Box::new(FixedClock(NOW)),
        files: Box::new(SystemFiles { cwd: sandbox.text() }),
        lock: LockTimings::default(),
        process: Box::new(SystemProcess { env: env.clone() }),
        git: Box::new(NoGit),
        env,
        defaults: String::new(),
        defaults_path: "defaults.json".to_string(),
        routing: crate::SHIPPED_ROUTING.to_string(),
    };

    let first = list_client(&runtime, "vibe");
    assert_eq!(
        first,
        Some(ClientListing::Models(vec![named("glm-5-3", &["glm-5-3-latest"]), model("mistral-medium-3.5")]))
    );
    let cache = cache_home.join("model-gateway").join("vibe-available.json");
    let kept = std::fs::read_to_string(&cache).expect("the answer is kept");
    let order = [
        "{\"binary\":\"",
        ",\"binary_ctime_ms\":",
        ",\"binary_ino\":",
        ",\"binary_size\":",
        ",\"config_path\":\"",
        ",\"config_sha256\":\"",
        ",\"models\":[\"glm-5-3\",\"mistral-medium-3.5\"]",
        ",\"version\":2",
        &format!(",\"written_at_ms\":{NOW}}}"),
    ];
    let mut at = 0;
    for part in order {
        let found = kept[at..].find(part).unwrap_or_else(|| panic!("{part} is not in {kept}"));
        at += found + part.len();
    }
    assert_eq!(std::fs::metadata(&cache).expect("the answer file").permissions().mode() & 0o777, 0o600);
    let directory = cache.parent().expect("the answer's directory");
    assert_eq!(std::fs::metadata(directory).expect("the answer's directory").permissions().mode() & 0o777, 0o700);

    // The answer is kept, so the call that follows reads it instead of starting Vibe again.
    assert_eq!(list_client(&runtime, "vibe"), first);
    assert_eq!(std::fs::read_to_string(&marker).expect("the probe ran").lines().count(), 1);
}
