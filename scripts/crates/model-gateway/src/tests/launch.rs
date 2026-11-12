//! Tests for the launchers, the in-process launch path, Vibe's sign-in check
//! and the home's MCP servers, ported from the node runner's tests of the same
//! modules. Every client is a fake: nothing here starts a real program.

use crate::client_run::{require_supported_extras, ClientRun, ClientRunError, ClientRunRequest};
use crate::error::GatewayError;
use crate::extras::{AttachedServer, RunExtras, SandboxProfile};
use crate::home_mcp::home_mcp_servers;
use crate::json::{self, Value};
use crate::launchers::{launcher, LaunchExtras, LaunchPaths, LaunchRequest, DELEGATION_MARK, READ_ONLY_AGENT, VIBE_ENV, WRITE_AGENT};
use crate::ports::{FileStamp, Files, FsError, Process, ProcessCommand, ProcessResult};
use crate::run::{RunClient, RunMode, NO_TOKENS};
use crate::vibe_sign_in::{keyring_query, node_platform, node_platform_of, vibe_signed_in};
use std::sync::Mutex;
use std::collections::{BTreeMap, HashMap};

const HOME: &str = "/home/test";
const CONFIG: &str = "/home/test/.vibe/config.toml";
const WORK: &str = "/run/work";

/// Files held in memory, with paths a test makes fail to look at or to read.
#[derive(Default)]
struct Memory {
    files: HashMap<String, String>,
    exists_fails: Vec<String>,
    read_fails: Vec<String>,
}

impl Memory {
    fn with(mut self, path: &str, content: &str) -> Memory {
        self.files.insert(path.to_string(), content.to_string());
        self
    }

    fn unlookable(mut self, path: &str) -> Memory {
        self.exists_fails.push(path.to_string());
        self
    }

    fn unreadable(mut self, path: &str) -> Memory {
        self.read_fails.push(path.to_string());
        self
    }
}

fn denied(path: &str) -> FsError {
    FsError { code: "EACCES".to_string(), message: format!("EACCES: permission denied, access '{path}'") }
}

impl Files for Memory {
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
        if self.read_fails.iter().any(|failing| failing == path) {
            return Err(denied(path));
        }
        self.files.get(path).cloned().ok_or(FsError { code: "ENOENT".to_string(), message: format!("ENOENT: no such file or directory, open '{path}'") })
    }

    fn write_atomic(&self, _path: &str, _content: &str) -> Result<(), FsError> {
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
        if self.exists_fails.iter().any(|failing| failing == path) {
            return Err(denied(path));
        }
        Ok(self.files.contains_key(path))
    }

    fn append(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn stamp(&self, _path: &str) -> Option<FileStamp> {
        None
    }

    fn write_private(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
}

/// A process that records every launch and answers them from a queue, or by the program's name.
#[derive(Default)]
struct Fake {
    ran: Mutex<Vec<ProcessCommand>>,
    queue: Mutex<Vec<Result<ProcessResult, String>>>,
    by_program: HashMap<String, Result<ProcessResult, String>>,
}

impl Fake {
    fn answering(stdout: &str) -> Fake {
        Fake { queue: Mutex::new(vec![Ok(finished(stdout, 0))]), ..Fake::default() }
    }

    fn failing(error: &str) -> Fake {
        Fake { queue: Mutex::new(vec![Err(error.to_string())]), ..Fake::default() }
    }

    fn program(mut self, program: &str, answer: Result<ProcessResult, String>) -> Fake {
        self.by_program.insert(program.to_string(), answer);
        self
    }

    fn first(&self) -> ProcessCommand {
        self.ran.lock().expect("the launches")[0].clone()
    }

    fn count(&self) -> usize {
        self.ran.lock().expect("the launches").len()
    }
}

impl Process for Fake {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Err("a launch goes through run".to_string())
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.ran.lock().expect("the launches").push(command.clone());
        if let Some(answer) = self.by_program.get(&command.command) {
            return answer.clone();
        }
        self.queue.lock().expect("the answers").remove(0)
    }
}

fn finished(stdout: &str, exit_code: i32) -> ProcessResult {
    ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code, timed_out: false }
}

fn env() -> HashMap<String, String> {
    HashMap::from([("HOME".to_string(), HOME.to_string())])
}

fn paths() -> LaunchPaths {
    LaunchPaths { export_dir: "/run/export".to_string(), prompt_path: "/run/prompt.md".to_string(), work_dir: WORK.to_string(), ..LaunchPaths::default() }
}

fn launch_request(model: &str) -> LaunchRequest {
    LaunchRequest { model: model.to_string(), time_limit_s: 900.0, max_price_usd: None, mode: None, provider: None }
}

/// The request a test sends unless it says otherwise: one Vibe launch of a critic's model.
fn request(client: RunClient, binary: &str, model: &str, extras: RunExtras) -> ClientRunRequest {
    ClientRunRequest { launch: launch_request(model), binary: binary.to_string(), client, prompt: "read this".to_string(), paths: paths(), extras }
}

fn vibe(extras: RunExtras) -> ClientRunRequest {
    request(RunClient::Vibe, "vibe", "glm-5-3", extras)
}

fn launch(files: &Memory, process: &Fake, request: &ClientRunRequest) -> Result<crate::client_run::ClientRunOutcome, ClientRunError> {
    ClientRun { process, files, env: &env() }.launch(request)
}

fn gateway(error: ClientRunError) -> GatewayError {
    match error {
        ClientRunError::Gateway(error) => error,
        ClientRunError::Spawn(words) => panic!("expected a refusal, got a spawn failure: {words}"),
    }
}

/// The value of one flag of a launch.
fn flag(command: &ProcessCommand, name: &str) -> Option<String> {
    command.args.iter().position(|arg| arg == name).map(|at| command.args[at + 1].clone())
}

/// The values of every occurrence of one flag of a launch, in the order it names them.
fn flags(command: &ProcessCommand, name: &str) -> Vec<String> {
    command.args.iter().enumerate().filter(|(at, _)| *at > 0 && command.args[*at - 1] == name).map(|(_, arg)| arg.clone()).collect()
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

fn server(name: &str, args: &[&str], command: &str) -> AttachedServer {
    AttachedServer { name: name.to_string(), command: command.to_string(), args: strings(args) }
}

fn home_with_servers() -> Memory {
    Memory::default().with(
        CONFIG,
        "[[mcp_servers]]\nname = \"hindsight\"\ntransport = \"stdio\"\ncommand = \"hindsight\"\n\n[[mcp_servers]]\nname = \"packet\"\ntransport = \"stdio\"\ncommand = \"rm\"\n",
    )
}

/// The extras a caller that starts its own search server sends: the server, the home off, the agent and the tools.
fn search_extras() -> RunExtras {
    RunExtras {
        agent: Some("critic-glm".to_string()),
        disable_home_mcp: Some(true),
        env: Some(BTreeMap::from([("VIBE_ENABLE_CONNECTORS".to_string(), "false".to_string())])),
        mcp_servers: Some(vec![server("packet", &["server.mjs", "--root", "/p"], "node")]),
        tools: Some(strings(&["read_file", "packet_grep"])),
        ..RunExtras::default()
    }
}

fn vibe_servers_of(command: &ProcessCommand) -> Vec<String> {
    let servers = json::parse(command.env.get("VIBE_MCP_SERVERS").expect("the servers")).expect("JSON");
    let Value::Arr(servers) = servers else { panic!("an array") };
    servers.iter().map(|server| server.as_object().and_then(|table| table.get("name")).and_then(Value::as_str).expect("a name").to_string()).collect()
}

#[test]
fn attaches_the_servers_the_caller_names_switches_the_homes_off_and_enables_the_tools_it_names() {
    let files = home_with_servers();
    let process = Fake::answering("{\"type\":\"assistant\",\"content\":\"fine\"}\n");

    let outcome = launch(&files, &process, &vibe(search_extras())).expect("the launch");

    assert_eq!(outcome.result.exit_code, 0);
    let command = process.first();
    assert_eq!(command.command, "env");
    assert_eq!(command.args[..5], strings(&["-u", "MISTRAL_API_KEY", "-u", "VIBE_HOME", "vibe"])[..]);
    assert_eq!(flag(&command, "--agent").as_deref(), Some("critic-glm"));
    assert_eq!(flags(&command, "--enabled-tools"), strings(&["read_file", "packet_grep"]));
    let servers = concat!(
        r#"[{"args":["server.mjs","--root","/p"],"command":"node","cwd":"/run/work","name":"packet","startup_timeout_sec":10,"tool_timeout_sec":30,"transport":"stdio"},"#,
        r#"{"command":"false","disabled":true,"name":"hindsight","transport":"stdio"}]"#
    );
    let expected: BTreeMap<String, String> = [
        ("VIBE_ENABLE_CONNECTORS", "false"),
        ("VIBE_ENABLE_MCP", "true"),
        ("VIBE_ENABLE_SUBAGENTS", "false"),
        ("VIBE_INCLUDE_PROJECT_CONTEXT", "false"),
        ("VIBE_MCP_SERVERS", servers),
        ("VIBE_OUTSIDE_WORKDIR", "deny"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value.to_string()))
    .collect();
    assert_eq!(command.env, expected);
    assert_eq!(command.cwd.as_deref(), Some(WORK));
    assert_eq!(command.timeout_ms, Some(960_000));
    assert_eq!(command.stdin, None);
}

#[test]
fn leaves_the_model_to_the_agent_a_caller_names_since_the_agents_file_pins_one() {
    let process = Fake::answering("{}");
    let extras = RunExtras { agent: Some("critic-glm".to_string()), ..RunExtras::default() };

    launch(&Memory::default(), &process, &vibe(extras)).expect("the launch");

    let command = process.first();
    assert!(!command.args.contains(&"--model".to_string()));
    assert!(!command.args.contains(&"glm-5-3".to_string()));
    assert_eq!(flag(&command, "--agent").as_deref(), Some("critic-glm"));
}

#[test]
fn still_names_the_model_the_run_was_checked_for_beside_the_read_only_agent_when_no_agent_is_named() {
    let process = Fake::answering("{}");
    let extras = RunExtras { tools: Some(strings(&["read_file"])), ..RunExtras::default() };

    launch(&Memory::default(), &process, &vibe(extras)).expect("the launch");

    let command = process.first();
    assert_eq!(flag(&command, "--model").as_deref(), Some("glm-5-3"));
    assert_eq!(flag(&command, "--agent").as_deref(), Some(READ_ONLY_AGENT));
}

#[test]
fn leaves_the_homes_servers_alone_when_the_caller_does_not_switch_them_off() {
    let process = Fake::answering("{}");
    let extras = RunExtras { mcp_servers: Some(vec![server("packet", &[], "node")]), ..RunExtras::default() };

    launch(&home_with_servers(), &process, &vibe(extras)).expect("the launch");

    assert_eq!(vibe_servers_of(&process.first()), strings(&["packet"]));
}

#[test]
fn refuses_to_switch_the_home_off_when_its_configuration_does_not_parse_naming_the_file_and_the_error() {
    let process = Fake::answering("{}");
    let files = Memory::default().with(CONFIG, "[[mcp_servers]\n");
    let extras = RunExtras { disable_home_mcp: Some(true), mcp_servers: Some(vec![server("packet", &[], "node")]), ..RunExtras::default() };

    let error = gateway(launch(&files, &process, &vibe(extras)).expect_err("a refusal"));

    assert_eq!(error.code, "home-config-unreadable");
    assert_eq!(error.message, format!("{CONFIG}: cannot read the home's MCP servers: line 1: expected '.'"));
    assert_eq!(process.count(), 0);
}

#[test]
fn refuses_to_switch_the_home_off_when_its_configuration_cannot_be_read() {
    let files = Memory::default().with(CONFIG, "theme = 1\n").unreadable(CONFIG);
    let error = home_mcp_servers(&files, &env()).expect_err("a refusal");
    assert_eq!(error.code, "home-config-unreadable");
    assert_eq!(error.message, format!("{CONFIG}: cannot read the home's MCP servers: EACCES: permission denied, access '{CONFIG}'"));
}

#[test]
fn a_home_that_cannot_be_looked_at_or_has_no_home_variable_is_not_read_as_empty() {
    let error = home_mcp_servers(&Memory::default().unlookable(CONFIG), &env()).expect_err("a failure");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("unexpected", "EACCES: permission denied, access '/home/test/.vibe/config.toml'"));
    let error = home_mcp_servers(&Memory::default(), &HashMap::new()).expect_err("a failure");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("unexpected", "required environment variable is missing: HOME"));
}

#[test]
fn switches_nothing_off_when_the_home_declares_no_configuration_servers_or_names() {
    let extras = || RunExtras { disable_home_mcp: Some(true), ..RunExtras::default() };
    for config in [None, Some("theme = \"dark\"\n"), Some("mcp_servers = [\"hindsight\", { command = \"hindsight\" }]\n")] {
        let process = Fake::answering("{}");
        let files = config.map_or_else(Memory::default, |text| Memory::default().with(CONFIG, text));
        launch(&files, &process, &vibe(extras())).expect("the launch");
        assert_eq!(process.first().env.get("VIBE_MCP_SERVERS"), None);
        assert_eq!(process.first().env.get("VIBE_ENABLE_MCP").map(String::as_str), Some("false"));
    }
}

#[test]
fn reads_a_home_that_declares_its_servers_as_one_inline_table() {
    let process = Fake::answering("{}");
    let files = Memory::default().with(CONFIG, "mcp_servers = { command = \"hindsight\", name = \"hindsight\" }\n");
    let extras = RunExtras { disable_home_mcp: Some(true), ..RunExtras::default() };

    launch(&files, &process, &vibe(extras)).expect("the launch");

    assert_eq!(vibe_servers_of(&process.first()), strings(&["hindsight"]));
}

#[test]
fn does_not_read_the_home_when_the_switch_is_false() {
    let process = Fake::answering("{}");
    let extras = RunExtras { disable_home_mcp: Some(false), ..RunExtras::default() };
    launch(&Memory::default().with(CONFIG, "[[mcp_servers]\n"), &process, &vibe(extras)).expect("the launch");
    assert_eq!(process.first().env.get("VIBE_MCP_SERVERS"), None);
}

#[test]
fn runs_antigravity_in_the_read_only_sandbox_profile_a_stream_the_schema_and_the_tools_it_may_use() {
    let process = Fake::answering(r#"{"type":"result","response":"looks fine","usage":{}}"#);
    let extras = RunExtras {
        sandbox: Some(SandboxProfile::ReadOnly),
        schema: Some("/assets/finding.schema.json".to_string()),
        tools: Some(strings(&["view_file"])),
        ..RunExtras::default()
    };

    let outcome = launch(&Memory::default(), &process, &request(RunClient::Agy, "agy", "gemini-3.8-flash-high", extras)).expect("the launch");

    assert_eq!(outcome.output.answer.as_deref(), Some("looks fine"));
    let deny = r#"{"permissions":{"deny":["mcp(*)","read_url(*)","execute_url(*)","command(*)","write_file(*)","list_dir(*)","grep_search(*)","find_by_name(*)"]}}"#;
    assert_eq!(
        process.first().args,
        strings(&[
            "-p",
            "read this",
            "--output-format",
            "stream-json",
            "--model",
            "gemini-3.8-flash-high",
            "--print-timeout",
            "900s",
            "--disable-slash-commands",
            "--sandbox",
            "--json-schema",
            "/assets/finding.schema.json",
            "--settings",
            deny,
        ])
    );
    assert_eq!(process.first().command, "agy");
    assert_eq!(process.first().timeout_ms, Some(960_000));
}

#[test]
fn denies_only_the_tool_families_a_searching_antigravity_run_does_not_name() {
    let process = Fake::answering("{}");
    let extras = RunExtras { sandbox: Some(SandboxProfile::ReadOnly), tools: Some(strings(&["view_file", "list_dir", "grep_search", "find_by_name"])), ..RunExtras::default() };

    launch(&Memory::default(), &process, &request(RunClient::Agy, "agy", "gemini-3.8-flash-high", extras)).expect("the launch");

    let command = process.first();
    assert_eq!(flag(&command, "--settings").as_deref(), Some(r#"{"permissions":{"deny":["mcp(*)","read_url(*)","execute_url(*)","command(*)","write_file(*)"]}}"#));
    assert_eq!(flag(&command, "--json-schema"), None);
}

#[test]
fn runs_antigravity_without_the_sandbox_as_one_json_object_with_the_effort_its_model_names() {
    let read = Fake::answering("{}");
    let write = Fake::answering("{}");
    let plain = || request(RunClient::Agy, "agy", "gemini-3.8-flash-high", RunExtras::default());

    launch(&Memory::default(), &read, &plain()).expect("the launch");
    let mut writing = plain();
    writing.launch.mode = Some(RunMode::Write);
    launch(&Memory::default(), &write, &writing).expect("the launch");

    assert_eq!(
        read.first().args,
        strings(&["--print", "read this", "--model", "gemini-3.8-flash-high", "--output-format", "json", "--effort", "high", "--mode", "plan", "--print-timeout", "900s"])
    );
    assert_eq!(flag(&write.first(), "--mode").as_deref(), Some("accept-edits"));
}

#[test]
fn takes_the_effort_from_the_last_word_of_the_model_id_and_defaults_to_medium() {
    for (model, effort) in [("gemini-3.8-flash-low", "low"), ("gemini-3.8-flash-medium", "medium"), ("gemini-3.8-flash-high", "high"), ("gemini-3.8-flash", "medium"), ("high", "medium")] {
        let process = Fake::answering("");
        launch(&Memory::default(), &process, &request(RunClient::Codex, "codex", model, RunExtras::default())).expect("the launch");
        assert_eq!(flag(&process.first(), "-c"), Some(format!("model_reasoning_effort=\"{effort}\"")), "{model}");
    }
}

#[test]
fn refuses_an_extra_the_client_cannot_apply_naming_the_extra_and_the_client() {
    let process = Fake::answering("{}");
    let sandbox = RunExtras { sandbox: Some(SandboxProfile::ReadOnly), ..RunExtras::default() };
    let error = gateway(launch(&Memory::default(), &process, &request(RunClient::Claude, "claude", "m", sandbox)).expect_err("a refusal"));
    assert_eq!((error.code.as_str(), error.message.as_str()), ("extras-not-supported", "claude cannot apply the sandbox extra"));

    let servers = RunExtras { mcp_servers: Some(Vec::new()), ..RunExtras::default() };
    let error = gateway(launch(&Memory::default(), &process, &request(RunClient::Agy, "agy", "m", servers)).expect_err("a refusal"));
    assert_eq!(error.message, "agy cannot apply the mcp_servers extra");
    assert_eq!(process.count(), 0);
}

#[test]
fn refuses_an_extra_its_launcher_would_drop_a_schema_for_vibe_a_home_switch_for_antigravity() {
    let process = Fake::answering("{}");
    let schema = RunExtras { schema: Some("/assets/finding.schema.json".to_string()), ..RunExtras::default() };
    let error = gateway(launch(&Memory::default(), &process, &vibe(schema)).expect_err("a refusal"));
    assert_eq!(error.message, "vibe cannot apply the schema extra");

    let home = RunExtras { disable_home_mcp: Some(true), ..RunExtras::default() };
    let error = gateway(launch(&Memory::default(), &process, &request(RunClient::Agy, "agy", "m", home)).expect_err("a refusal"));
    assert_eq!(error.message, "agy cannot apply the disable_home_mcp extra");
    assert_eq!(process.count(), 0);
}

#[test]
fn refuses_a_vibe_run_that_needs_a_web_tool_before_launching_anything_naming_the_tool() {
    let process = Fake::answering("{}");
    for tool in ["web_fetch", "web_search"] {
        let extras = RunExtras { tools: Some(strings(&["read_file", tool])), ..RunExtras::default() };
        let error = gateway(launch(&Memory::default(), &process, &vibe(extras)).expect_err("a refusal"));
        assert_eq!(error.code, "extras-not-supported");
        assert!(error.message.starts_with(&format!("vibe cannot enable the {tool} tool: it loads on demand")), "{}", error.message);
    }
    assert_eq!(process.count(), 0);
    launch(&Memory::default(), &process, &vibe(RunExtras { tools: Some(strings(&["read_file"])), ..RunExtras::default() })).expect("another tool goes through");
    assert_eq!(process.count(), 1);
}

#[test]
fn refuses_the_tools_and_the_schema_an_antigravity_run_applies_only_in_the_read_only_sandbox() {
    let tools = RunExtras { tools: Some(strings(&["view_file"])), ..RunExtras::default() };
    let schema = RunExtras { schema: Some("/assets/finding.schema.json".to_string()), ..RunExtras::default() };
    for (extras, name) in [(tools, "tools"), (schema, "schema")] {
        let error = require_supported_extras(RunClient::Agy, &extras).expect_err("a refusal");
        assert_eq!(error.code, "extras-not-supported");
        assert_eq!(error.message, format!("agy applies the {name} extra only in the read-only sandbox, so it needs the sandbox extra too"));
    }
}

#[test]
fn refuses_an_environment_variable_the_launcher_sets_itself_naming_the_variable() {
    for variable in ["VIBE_OUTSIDE_WORKDIR", "MISTRAL_API_KEY"] {
        let extras = RunExtras { env: Some(BTreeMap::from([(variable.to_string(), "x".to_string())])), ..RunExtras::default() };
        let error = require_supported_extras(RunClient::Vibe, &extras).expect_err("a refusal");
        assert_eq!(error.message, format!("vibe sets {variable} itself, so extras.env may not name it"));
    }
    // A client whose launcher sets nothing itself takes any variable, and one without extras passes.
    let free = RunExtras { env: Some(BTreeMap::from([("MISTRAL_API_KEY".to_string(), "x".to_string())])), ..RunExtras::default() };
    assert_eq!(require_supported_extras(RunClient::Claude, &free), Ok(()));
    assert_eq!(require_supported_extras(RunClient::Vibe, &RunExtras::default()), Ok(()));
}

#[test]
fn checks_every_extra_name_against_the_client() {
    let everything = RunExtras {
        agent: Some("a".to_string()),
        disable_home_mcp: Some(true),
        env: Some(BTreeMap::new()),
        mcp_servers: Some(Vec::new()),
        sandbox: Some(SandboxProfile::ReadOnly),
        schema: Some("s".to_string()),
        tools: Some(Vec::new()),
    };
    // Each client's first refusal is the first extra it cannot apply, in the order the names are listed.
    for (client, first) in [
        (RunClient::Vibe, "sandbox"),
        (RunClient::Agy, "agent"),
        (RunClient::Claude, "agent"),
        (RunClient::Opencode, "agent"),
        (RunClient::Hermes, "agent"),
        (RunClient::Codex, "agent"),
    ] {
        let error = require_supported_extras(client, &everything).expect_err("a refusal");
        assert_eq!(error.message, format!("{} cannot apply the {first} extra", client.as_str()));
    }
    for extra in [
        RunExtras { agent: Some("a".to_string()), ..RunExtras::default() },
        RunExtras { disable_home_mcp: Some(true), ..RunExtras::default() },
        RunExtras { mcp_servers: Some(Vec::new()), ..RunExtras::default() },
        RunExtras { sandbox: Some(SandboxProfile::ReadOnly), ..RunExtras::default() },
    ] {
        assert!(require_supported_extras(RunClient::Hermes, &extra).is_err());
    }
}

#[test]
fn reads_the_export_vibe_wrote_and_hands_it_to_the_output() {
    let export = r#"{"cost_usd":0.25,"usage":{"cached_input_tokens":1,"input_tokens":2,"output_tokens":3}}"#;
    let files = Memory::default().with("/run/export/export.json", export);
    let process = Fake::answering("[{\"content\":[{\"text\":\"done\"}],\"role\":\"assistant\",\"type\":\"message\"}]");

    let outcome = launch(&files, &process, &vibe(RunExtras::default())).expect("the launch");

    assert_eq!(outcome.exported, Some(json::parse(export).expect("JSON")));
    assert_eq!(outcome.output.answer.as_deref(), Some("done"));
    assert_eq!(outcome.output.charged, Some(0.25));
}

#[test]
fn reads_no_export_when_the_client_wrote_none_or_it_does_not_parse_or_cannot_be_read() {
    for files in [Memory::default(), Memory::default().with("/run/export/export.json", "{oops"), Memory::default().with("/run/export/export.json", "{}").unreadable("/run/export/export.json")] {
        let outcome = launch(&files, &Fake::answering("x"), &vibe(RunExtras::default())).expect("the launch");
        assert_eq!(outcome.exported, None);
        assert_eq!(outcome.output.tokens, NO_TOKENS);
    }
    let error = gateway(launch(&Memory::default().unlookable("/run/export/export.json"), &Fake::answering("x"), &vibe(RunExtras::default())).expect_err("a failure"));
    assert_eq!(error.code, "unexpected");
}

#[test]
fn reports_a_program_that_will_not_start_and_the_result_of_one_that_exits_badly() {
    let error = launch(&Memory::default(), &Fake::failing("spawn vibe ENOENT"), &vibe(RunExtras::default())).expect_err("a spawn failure");
    assert_eq!(error, ClientRunError::Spawn("spawn vibe ENOENT".to_string()));

    let bad = Fake { queue: Mutex::new(vec![Ok(finished("", 3))]), ..Fake::default() };
    let outcome = launch(&Memory::default(), &bad, &vibe(RunExtras::default())).expect("the launch");
    assert_eq!(outcome.result.exit_code, 3);
}

#[test]
fn runs_a_writing_vibe_run_through_the_writing_agent_with_the_price_cap() {
    let process = Fake::answering("");
    let mut writing = vibe(RunExtras::default());
    writing.launch.mode = Some(RunMode::Write);
    writing.launch.max_price_usd = Some(0.5);
    writing.launch.time_limit_s = 30.5;

    launch(&Memory::default(), &process, &writing).expect("the launch");

    let command = process.first();
    assert_eq!(flag(&command, "--agent").as_deref(), Some(WRITE_AGENT));
    assert_eq!(flag(&command, "--max-price").as_deref(), Some("0.5"));
    assert_eq!(flag(&command, "--time-limit").as_deref(), Some("30.5"));
    assert_eq!(flag(&command, "--output-dir").as_deref(), Some("/run/export"));
    assert_eq!(flag(&command, "--prompt-file").as_deref(), Some("/run/prompt.md"));
    assert_eq!(flag(&command, "--workdir").as_deref(), Some(WORK));
    assert_eq!(command.timeout_ms, Some(90_500));
    assert_eq!(VIBE_ENV.len(), 4);
}

#[test]
fn runs_claude_with_the_prompt_on_stdin_and_the_run_limit_as_the_only_timer() {
    let read = Fake::answering(r#"{"type":"result","result":"fine"}"#);
    let outcome = launch(&Memory::default(), &read, &request(RunClient::Claude, "claude", "sonnet", RunExtras::default())).expect("the launch");
    assert_eq!(outcome.output.answer.as_deref(), Some("fine"));
    let command = read.first();
    assert_eq!(command.command, "claude");
    assert_eq!(command.args, strings(&["-p", "--model", "sonnet", "--output-format", "json", "--permission-mode", "plan", "--allowedTools", "Read,Grep,Glob"]));
    assert_eq!(command.stdin.as_deref(), Some("read this"));
    assert_eq!(command.timeout_ms, Some(900_000));

    let write = Fake::answering("");
    let mut writing = request(RunClient::Claude, "claude", "sonnet", RunExtras::default());
    writing.launch.mode = Some(RunMode::Write);
    launch(&Memory::default(), &write, &writing).expect("the launch");
    assert_eq!(flag(&write.first(), "--permission-mode").as_deref(), Some("acceptEdits"));
    assert_eq!(flag(&write.first(), "--allowedTools").as_deref(), Some("Read,Grep,Glob,Edit,Write"));
}

#[test]
fn runs_opencode_standalone_with_the_prompt_as_its_message() {
    let process = Fake::answering("");
    let extras = RunExtras { env: Some(BTreeMap::from([("X".to_string(), "1".to_string())])), ..RunExtras::default() };

    launch(&Memory::default(), &process, &request(RunClient::Opencode, "opencode", "opencode-go/kimi", extras)).expect("the launch");

    let command = process.first();
    assert_eq!(command.args, strings(&["run", "--standalone", "--model", "opencode-go/kimi", "--format", "json", "read this"]));
    assert_eq!(command.env, BTreeMap::from([("X".to_string(), "1".to_string())]));
    assert_eq!(command.timeout_ms, Some(900_000));
    assert_eq!(command.stdin, None);
}

#[test]
fn runs_hermes_with_its_provider_and_reads_the_usage_file_from_the_run_directory_or_the_export_directory() {
    let usage = r#"{"cost":0.5,"input_tokens":4,"output_tokens":5}"#;
    let mut named = request(RunClient::Hermes, "hermes", "deepseek", RunExtras::default());
    named.launch.provider = Some("deepinfra".to_string());
    named.paths.run_dir = Some("/run/own".to_string());
    let files = Memory::default().with("/run/own/usage.json", usage);
    let process = Fake::answering("an answer");

    let outcome = launch(&files, &process, &named).expect("the launch");

    let command = process.first();
    assert_eq!(command.args, strings(&["-z", "read this", "--usage-file", "/run/own/usage.json", "-m", "deepseek", "--provider", "deepinfra"]));
    assert_eq!(command.timeout_ms, Some(960_000));
    assert_eq!(outcome.exported, Some(json::parse(usage).expect("JSON")));

    // Without a run directory the report is in the export directory, and a run that names no provider passes the word node's template makes of it.
    let bare = request(RunClient::Hermes, "hermes", "deepseek", RunExtras::default());
    let process = Fake::answering("an answer");
    launch(&Memory::default().with("/run/export/usage.json", usage), &process, &bare).expect("the launch");
    assert_eq!(flag(&process.first(), "--usage-file").as_deref(), Some("/run/export/usage.json"));
    assert_eq!(flag(&process.first(), "--provider").as_deref(), Some("undefined"));
}

/// The git directories a writing run names: the worktree's own, and the repository's shared ones.
fn git_writable() -> Vec<String> {
    strings(&["/repo/.git/worktrees/run-1", "/repo/.git/objects", "/repo/.git/refs", "/repo/.git/logs"])
}

fn with_git_writable(mut request: ClientRunRequest) -> ClientRunRequest {
    request.paths.git_writable = Some(git_writable());
    request
}

#[test]
fn adds_each_git_directory_right_after_the_work_directory_so_the_sandbox_can_commit_in_the_worktree() {
    let process = Fake::answering("");
    let mut writing = with_git_writable(request(RunClient::Codex, "codex", "gpt", RunExtras::default()));
    writing.launch.mode = Some(RunMode::Write);

    launch(&Memory::default(), &process, &writing).expect("the launch");

    let command = process.first();
    let at = command.args.iter().position(|arg| arg == "-C").expect("-C");
    assert_eq!(command.args[at + 1], WORK);
    let expected: Vec<String> = git_writable().iter().flat_map(|dir| ["--add-dir".to_string(), dir.clone()]).collect();
    assert_eq!(command.args[at + 2..], expected[..]);
    assert_eq!(flags(&command, "--add-dir"), git_writable());
    assert_eq!(flag(&command, "-s").as_deref(), Some("workspace-write"));
    assert_eq!(command.stdin.as_deref(), Some("read this"));
    assert_eq!(command.timeout_ms, Some(900_000));
}

#[test]
fn adds_no_git_directory_for_a_read_only_run_or_a_writing_run_that_names_none_or_another_client() {
    let process = Fake::answering("");
    launch(&Memory::default(), &process, &with_git_writable(request(RunClient::Codex, "codex", "gpt", RunExtras::default()))).expect("the launch");
    assert_eq!(flags(&process.first(), "--add-dir"), Vec::<String>::new());
    assert_eq!(flag(&process.first(), "-s").as_deref(), Some("read-only"));

    let process = Fake::answering("");
    let mut writing = request(RunClient::Codex, "codex", "gpt", RunExtras::default());
    writing.launch.mode = Some(RunMode::Write);
    launch(&Memory::default(), &process, &writing).expect("the launch");
    assert_eq!(flags(&process.first(), "--add-dir"), Vec::<String>::new());
    assert_eq!(flag(&process.first(), "-s").as_deref(), Some("workspace-write"));

    let process = Fake::answering(r#"{"type":"result","result":"fine"}"#);
    let mut claude = with_git_writable(request(RunClient::Claude, "claude", "sonnet", RunExtras::default()));
    claude.launch.mode = Some(RunMode::Write);
    launch(&Memory::default(), &process, &claude).expect("the launch");
    assert!(!process.first().args.contains(&"--add-dir".to_string()));
    assert!(!process.first().args.contains(&git_writable()[0]));
}

#[test]
fn every_client_has_a_launcher_that_builds_its_command_and_reads_its_output() {
    let request = launch_request("m");
    let extras = LaunchExtras::default();
    for client in RunClient::ALL {
        let launcher = launcher(client);
        let command = (launcher.command)("bin", &request, &paths(), "p", Some(&extras));
        assert!(!command.command.is_empty());
        let nothing = (launcher.read)("", None);
        assert_eq!(nothing.answer, None, "{client:?}");
        // Building a command without any extras is the same for the clients that apply none.
        let bare = (launcher.command)("bin", &request, &paths(), "p", None);
        assert_eq!(bare.cwd.as_deref(), Some(WORK));
        assert_eq!(launcher.report_path.is_some(), matches!(client, RunClient::Vibe | RunClient::Hermes));
    }
    assert_eq!(DELEGATION_MARK, "AGENT_KIT_DELEGATION");
    let vibe_bare = (launcher(RunClient::Vibe).command)("vibe", &request, &paths(), "p", None);
    assert_eq!(vibe_bare.env.len(), 4);
    assert_eq!(launcher(RunClient::Vibe).report_path.map(|path| path(&paths())).as_deref(), Some("/run/export/export.json"));
}

fn found(stdout: &str) -> Result<ProcessResult, String> {
    Ok(finished(stdout, 0))
}

fn no_items() -> Result<ProcessResult, String> {
    found("(@ao [], @ao [])\n")
}

/// No real key: the fakes answer the keyring lookups, and the `.env` holds a placeholder.
fn signed_in(platform: &str, keyring: Option<(&str, Result<ProcessResult, String>)>, dotenv: Option<&str>) -> (bool, Vec<ProcessCommand>) {
    let mut process = Fake::default();
    if let Some((program, answer)) = keyring {
        process = process.program(program, answer);
    }
    let files = dotenv.map_or_else(Memory::default, |text| Memory::default().with("/h/.vibe/.env", text));
    let answer = vibe_signed_in(&process, &files, "/h/.vibe", platform).expect("the check");
    let ran = process.ran.lock().expect("the launches").clone();
    (answer, ran)
}

#[test]
fn counts_the_keychain_item_on_macos_asked_without_printing_it() {
    let (answer, ran) = signed_in("darwin", Some(("security", found("keychain: \"/Users/me/Library/Keychains/login.keychain-db\"\n"))), None);
    assert!(answer);
    assert_eq!(ran, vec![ProcessCommand::new("security", &["find-generic-password", "-s", "ai.mistral.vibe", "-a", "MISTRAL_API_KEY"])]);
    assert!(!ran[0].args.contains(&"-w".to_string()));
}

#[test]
fn falls_back_to_the_dotenv_on_macos_when_the_keychain_has_no_item() {
    let missing = || Ok(ProcessResult { stdout: String::new(), stderr: "not found".to_string(), exit_code: 44, timed_out: false });
    assert!(!signed_in("darwin", Some(("security", missing())), None).0);
    assert!(signed_in("darwin", Some(("security", missing())), Some("MISTRAL_API_KEY='placeholder'\n")).0);
}

#[test]
fn counts_an_unlocked_or_a_locked_secret_service_item_on_linux() {
    let (answer, ran) = signed_in("linux", Some(("gdbus", found("([objectpath '/org/freedesktop/secrets/collection/login/3'], @ao [])\n"))), None);
    assert!(answer);
    let mut expected = ProcessCommand::new(
        "gdbus",
        &[
            "call",
            "--session",
            "--timeout",
            "5",
            "--dest",
            "org.freedesktop.secrets",
            "--object-path",
            "/org/freedesktop/secrets",
            "--method",
            "org.freedesktop.Secret.Service.SearchItems",
            "{'service': 'ai.mistral.vibe', 'username': 'MISTRAL_API_KEY'}",
        ],
    );
    expected.timeout_ms = Some(6000);
    assert_eq!(ran, vec![expected]);
    assert!(signed_in("linux", Some(("gdbus", found("(@ao [], [objectpath '/org/freedesktop/secrets/collection/login/3'])\n"))), None).0);
}

#[test]
fn is_signed_out_on_linux_when_the_secret_service_has_no_item_and_there_is_no_dotenv_or_it_sets_nothing() {
    assert!(!signed_in("linux", Some(("gdbus", no_items())), None).0);
    assert!(!signed_in("linux", Some(("gdbus", no_items())), Some("MISTRAL_API_KEY=\nOTHER=x\n")).0);
    // A quoted path with nothing between the quotes is no item path.
    assert!(!signed_in("linux", Some(("gdbus", found("('/')\n"))), None).0);
    assert!(!signed_in("linux", Some(("gdbus", found("('/' ''/)\n"))), None).0);
}

#[test]
fn counts_the_dotenv_on_linux_when_the_secret_service_has_no_item_or_there_is_none_to_ask() {
    assert!(signed_in("linux", Some(("gdbus", no_items())), Some("MISTRAL_API_KEY=placeholder\n")).0);
    let no_service = Ok(ProcessResult { stdout: String::new(), stderr: "ServiceUnknown".to_string(), exit_code: 1, timed_out: false });
    assert!(!signed_in("linux", Some(("gdbus", no_service)), None).0);
    assert!(signed_in("linux", Some(("gdbus", Err("spawn gdbus ENOENT".to_string()))), Some("MISTRAL_API_KEY=\"placeholder\"\n")).0);
}

#[test]
fn reads_the_dotenv_line_the_way_the_pattern_does() {
    for (line, signed) in [
        ("MISTRAL_API_KEY=x", true),
        ("  MISTRAL_API_KEY = x", true),
        ("\tMISTRAL_API_KEY=\"x\"", true),
        ("MISTRAL_API_KEY='x", true),
        ("MISTRAL_API_KEY=", false),
        ("MISTRAL_API_KEY=   ", false),
        ("MISTRAL_API_KEY=''", false),
        ("MISTRAL_API_KEY=\"\"x", false),
        ("MISTRAL_API_KEY='", false),
        ("MISTRAL_API_KEY x", false),
        ("OTHER_MISTRAL_API_KEY=x", false),
        ("# MISTRAL_API_KEY=x", false),
        ("", false),
    ] {
        assert_eq!(signed_in("win32", None, Some(line)).0, signed, "{line:?}");
    }
    assert!(signed_in("win32", None, Some("OTHER=1\nMISTRAL_API_KEY=x\n")).0);
}

#[test]
fn asks_no_keyring_on_a_platform_it_does_not_know_the_keyring_of_so_only_the_dotenv_counts() {
    let (answer, ran) = signed_in("win32", None, Some("MISTRAL_API_KEY=placeholder\n"));
    assert!(answer);
    assert!(ran.is_empty());
    assert_eq!(keyring_query("win32"), None);
}

#[test]
fn times_the_secret_service_lookup_and_leaves_the_keychain_untimed() {
    assert!(keyring_query("linux").and_then(|query| query.timeout_ms).is_some_and(|ms| ms > 0));
    assert_eq!(keyring_query("darwin").and_then(|query| query.timeout_ms), None);
}

#[test]
fn a_dotenv_that_cannot_be_looked_at_or_read_is_a_failure_not_a_signed_out_home() {
    let process = Fake::default();
    let files = Memory::default().unlookable("/h/.vibe/.env");
    assert_eq!(vibe_signed_in(&process, &files, "/h/.vibe", "win32").expect_err("a failure").code, "unexpected");
    let files = Memory::default().with("/h/.vibe/.env", "MISTRAL_API_KEY=x").unreadable("/h/.vibe/.env");
    assert_eq!(vibe_signed_in(&process, &files, "/h/.vibe", "win32").expect_err("a failure").code, "unexpected");
}

#[test]
fn names_the_platform_the_way_node_does() {
    assert_eq!(node_platform_of("macos"), "darwin");
    assert_eq!(node_platform_of("windows"), "win32");
    assert_eq!(node_platform_of("linux"), "linux");
    assert_eq!(node_platform(), node_platform_of(std::env::consts::OS));
}
