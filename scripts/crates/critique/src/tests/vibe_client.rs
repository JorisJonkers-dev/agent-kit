//! The Vibe client: what one run's exit code, journal and export say, what it counts, and the launch that produces it.

use crate::clients::{ClientReadiness, CriticTokens, ModelAnswer, ModelAnswerReport, ModelQuestion};
use crate::dirs::SystemDirs;
use crate::error::CritiqueError;
use crate::git::SystemGit;
use crate::pricing::{TokenPrices, TokenUsage};
use crate::routing::{AgyCriticClient, ClaudeCriticClient, CodexCriticClient, GatewayModel, VibeCriticClient};
use crate::runtime::Runtime;
use crate::vibe_client::{
    answer_of, computed_cost, tokens_of, vibe_ask, vibe_charge, vibe_message, vibe_readiness, vibe_sign_in, ExpectedConfig, VibeMessage,
    VibeRun, SIGN_IN,
};
use model_gateway::adapters::clock::SystemClock;
use model_gateway::adapters::fs::SystemFiles;
use model_gateway::adapters::process::SystemProcess;
use model_gateway::json::{parse, Object, Value};
use model_gateway::ports::{FileStamp, Files, FsError, Process, ProcessCommand, ProcessResult};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A run that exited 0 with nothing to show.
fn run() -> VibeRun {
    VibeRun {
        exit_code: 0,
        expected: ExpectedConfig::Off,
        stdout: String::new(),
        stderr: String::new(),
        exported: None,
        timed_out: false,
        timeout_s: 120.0,
        cost_usd: Some(0.5),
        vibe_cost_usd: Some(0.5),
    }
}

fn json(text: &str) -> Value {
    parse(text).unwrap_or(Value::Null)
}

/// An export holding `usage`, built by hand for the counts JSON cannot spell.
fn export_with_usage(usage: Value) -> Value {
    let mut export = Object::new();
    export.set("usage", usage);
    Value::Obj(export)
}

/// A config that isolates the critic: MCP off, the agent's tools, no project context, no way out.
const ISOLATED: &str = r#"{"config":{"enable_mcp":false,"enabled_tools":["read_file","packet_grep","packet_list","packet_symbol"],"include_project_context":false,"outside_workdir":"deny"}}"#;

/// A journal whose last assistant message is `the answer`, with a non-text part between its text parts.
const JOURNAL: &str = r#"[{"type":"message","role":"user","content":[{"text":"review this"}]},{"type":"message","role":"assistant","content":[{"text":"the "},{"type":"tool"},{"text":"answer"}]}]"#;

fn failed(message: &VibeMessage) -> (String, String) {
    match message {
        VibeMessage::Failed { failure, detail, .. } => (failure.clone(), detail.clone()),
        VibeMessage::Ok { text, .. } => panic!("expected a failure, got {text}"),
    }
}

#[test]
fn a_timeout_is_the_outer_timers_failure() {
    let mut run = run();
    run.timed_out = true;
    run.timeout_s = 180.0;
    let (failure, detail) = failed(&vibe_message(&run));
    assert_eq!(failure, "timeout");
    assert_eq!(detail, "vibe did not return within 180s, so the outer timer killed it");
}

#[test]
fn a_documented_exit_code_names_its_failure() {
    for (code, failure) in [(1, "config-error"), (2, "infrastructure-failure"), (3, "stopped-early")] {
        let mut run = run();
        run.exit_code = code;
        run.stderr = "  bad config\n".to_string();
        let (named, detail) = failed(&vibe_message(&run));
        assert_eq!(named, failure);
        assert_eq!(detail, format!("vibe exited {code}: bad config"));
    }
}

#[test]
fn an_undocumented_exit_code_reads_exit_n() {
    let mut run = run();
    run.exit_code = 7;
    let (failure, detail) = failed(&vibe_message(&run));
    assert_eq!(failure, "exit-7");
    assert_eq!(detail, "vibe exited 7: ");
}

#[test]
fn an_exit_code_reads_the_exports_outcome_first() {
    let mut run = run();
    run.exit_code = 1;
    run.stderr = "ignored".to_string();
    run.exported = Some(json(r#"{"outcome":"deadline"}"#));
    let (_, detail) = failed(&vibe_message(&run));
    assert_eq!(detail, "vibe exited 1: outcome deadline");
}

#[test]
fn empty_stdout_is_empty_output() {
    let mut run = run();
    run.stdout = " \n\t".to_string();
    run.exported = Some(json(ISOLATED));
    let (failure, detail) = failed(&vibe_message(&run));
    assert_eq!(failure, "empty-output");
    assert_eq!(detail, "vibe exited 0 with empty stdout");
}

#[test]
fn a_run_without_an_export_is_missing_export() {
    let mut run = run();
    run.stdout = JOURNAL.to_string();
    let (failure, detail) = failed(&vibe_message(&run));
    assert_eq!(failure, "missing-export");
    assert_eq!(detail, "vibe wrote no readable export.json");
}

#[test]
fn every_isolation_violation_is_reported() {
    let mut run = run();
    run.stdout = JOURNAL.to_string();
    run.exported = Some(json(
        r#"{"config":{"enable_mcp":true,"enabled_tools":["read_file","bash"],"include_project_context":true,"outside_workdir":"allow"}}"#,
    ));
    let (failure, detail) = failed(&vibe_message(&run));
    assert_eq!(failure, "unsafe-config");
    assert_eq!(
        detail,
        "effective config: enable_mcp is not false; \
         enabled_tools is not read_file and the search server's tools (read_file, packet_grep, packet_list, packet_symbol); \
         include_project_context is not false, so the AGENTS.md instructions pointer reaches the critic; \
         outside_workdir is not deny"
    );
}

#[test]
fn a_config_without_a_config_object_violates_every_rule() {
    let mut run = run();
    run.stdout = JOURNAL.to_string();
    run.exported = Some(json(r#"{"config":"none"}"#));
    let (_, detail) = failed(&vibe_message(&run));
    assert!(detail.starts_with("effective config: enable_mcp is not false; enabled_tools is not read_file"));
    assert!(detail.ends_with("outside_workdir is not deny"));
}

#[test]
fn enabled_tools_must_be_the_agents_own() {
    for tools in [r#""read_file""#, r#"["packet_grep"]"#, r#"["read_file","bash"]"#, r#"["read_file",5]"#] {
        let mut run = run();
        run.stdout = JOURNAL.to_string();
        run.exported = Some(json(&format!(
            r#"{{"config":{{"enable_mcp":false,"enabled_tools":{tools},"include_project_context":false,"outside_workdir":"deny"}}}}"#
        )));
        let (failure, detail) = failed(&vibe_message(&run));
        assert_eq!(failure, "unsafe-config");
        assert_eq!(
            detail,
            "effective config: enabled_tools is not read_file and the search server's tools \
             (read_file, packet_grep, packet_list, packet_symbol)"
        );
    }
}

/// A searching critic's config, with `servers` as its `mcp_servers`.
fn search_config(servers: &str) -> Value {
    json(&format!(
        r#"{{"config":{{"enable_mcp":true,"mcp_servers":{servers},"enabled_tools":["read_file","packet_grep"],"include_project_context":false,"outside_workdir":"deny"}}}}"#
    ))
}

fn search_run(servers: &str) -> VibeRun {
    let mut run = run();
    run.stdout = JOURNAL.to_string();
    run.expected = ExpectedConfig::Search { command: "packet-search".to_string(), cwd: "/packet".to_string() };
    run.exported = Some(search_config(servers));
    run
}

#[test]
fn a_searching_critic_with_only_its_own_server_is_isolated() {
    let run = search_run(r#"[{"name":"packet","command":"packet-search","cwd":"/packet"},{"name":"other","disabled":true}]"#);
    assert_eq!(vibe_message(&run), VibeMessage::Ok { text: "the answer".to_string(), cost_usd: Some(0.5) });
}

#[test]
fn another_live_server_is_a_violation() {
    let run = search_run(r#"[{"name":"packet","command":"packet-search","cwd":"/packet"},{"name":"other"}]"#);
    let (_, detail) = failed(&vibe_message(&run));
    assert_eq!(detail, "effective config: MCP servers other than the search server are on: other");
}

#[test]
fn a_server_name_is_worded_as_javascript_words_it() {
    let run = search_run(
        r#"[{"name":5},{"name":null},{"name":["a",null]},{"name":{}},{},1,{"name":true},{"name":"other","disabled":true}]"#,
    );
    let (_, detail) = failed(&vibe_message(&run));
    assert_eq!(
        detail,
        "effective config: MCP servers other than the search server are on: 5, null, a,, [object Object], undefined, undefined, true; \
         the packet MCP server is not the runner's search server in this packet"
    );
}

#[test]
fn mcp_servers_that_are_not_a_list_leave_only_the_own_server_violation() {
    let run = search_run(r#""nope""#);
    let (_, detail) = failed(&vibe_message(&run));
    assert_eq!(detail, "effective config: the packet MCP server is not the runner's search server in this packet");
}

#[test]
fn the_own_server_must_be_the_runners_in_this_packet() {
    for servers in [
        r#"[{"name":"packet","command":"other","cwd":"/packet"}]"#,
        r#"[{"name":"packet","command":"packet-search","cwd":"/elsewhere"}]"#,
        r#"[{"name":"packet","command":"packet-search","cwd":"/packet"},{"name":"packet","command":"packet-search","cwd":"/packet"}]"#,
        r#"[{"name":"packet","command":5,"cwd":"/packet"}]"#,
    ] {
        let run = search_run(servers);
        let (_, detail) = failed(&vibe_message(&run));
        assert_eq!(detail, "effective config: the packet MCP server is not the runner's search server in this packet");
    }
}

#[test]
fn an_isolated_run_answers_with_its_last_assistant_message() {
    let mut run = run();
    run.stdout = JOURNAL.to_string();
    run.exported = Some(json(ISOLATED));
    assert_eq!(vibe_message(&run), VibeMessage::Ok { text: "the answer".to_string(), cost_usd: Some(0.5) });
}

#[test]
fn the_last_assistant_message_wins() {
    let mut run = run();
    run.stdout = r#"[{"type":"message","role":"assistant","content":[{"text":"first"}]},
        {"type":"effect","state":{"status":"ok"}},
        {"type":"message","role":"assistant","content":[{"text":"second"}]},
        {"type":"message","role":"user","content":[{"text":"third"}]}]"#
        .to_string();
    run.exported = Some(json(ISOLATED));
    assert_eq!(vibe_message(&run), VibeMessage::Ok { text: "second".to_string(), cost_usd: Some(0.5) });
}

#[test]
fn stdout_that_is_not_a_journal_is_unparseable() {
    for stdout in [
        "not json",
        r#"{"type":"message","role":"assistant","content":[{"text":"a"}]}"#,
        r#"[{"type":"message","role":"assistant"}]"#,
        r#"[{"type":"message","role":"assistant","content":"plain"}]"#,
        r#"[{"type":"message","role":"user","content":[{"text":"a"}]}]"#,
        r#"[5]"#,
    ] {
        let mut run = run();
        run.stdout = stdout.to_string();
        run.exported = Some(json(ISOLATED));
        let (failure, detail) = failed(&vibe_message(&run));
        assert_eq!(failure, "unparseable-output");
        assert_eq!(detail, "stdout is not a Vibe journal with an assistant message");
    }
}

#[test]
fn a_timed_out_run_charges_the_cap() {
    let mut run = run();
    run.timed_out = true;
    run.cost_usd = Some(0.2);
    assert_eq!(vibe_charge(&run, 1.0), 1.0);
}

#[test]
fn a_run_without_a_cost_charges_the_cap() {
    let mut run = run();
    run.cost_usd = None;
    assert_eq!(vibe_charge(&run, 1.0), 1.0);
}

#[test]
fn a_deadline_stop_charges_the_larger_of_cost_and_cap() {
    let mut run = run();
    run.exported = Some(json(r#"{"outcome":"deadline"}"#));
    run.cost_usd = Some(0.2);
    assert_eq!(vibe_charge(&run, 1.0), 1.0);
    run.cost_usd = Some(1.5);
    assert_eq!(vibe_charge(&run, 1.0), 1.5);
}

#[test]
fn a_zero_cost_for_tokens_used_charges_the_cap() {
    for usage in [r#"{"input_tokens":5}"#, r#"{"output_tokens":3}"#] {
        let mut run = run();
        run.exported = Some(json(&format!(r#"{{"usage":{usage}}}"#)));
        run.cost_usd = Some(0.0);
        assert_eq!(vibe_charge(&run, 1.0), 1.0);
    }
}

#[test]
fn a_zero_cost_without_tokens_charges_nothing() {
    for exported in [
        None,
        Some(json(r#"{"usage":{"input_tokens":0,"output_tokens":0}}"#)),
        Some(json(r#"{"usage":{"input_tokens":"5"}}"#)),
        Some(json(r#"{"usage":"none"}"#)),
    ] {
        let mut run = run();
        run.exported = exported;
        run.cost_usd = Some(0.0);
        assert_eq!(vibe_charge(&run, 1.0), 0.0);
    }
}

#[test]
fn a_priced_run_charges_its_cost() {
    let mut run = run();
    run.cost_usd = Some(0.3);
    assert_eq!(vibe_charge(&run, 1.0), 0.3);
}

fn prices() -> TokenPrices {
    TokenPrices { input: 2.0, cached_input: 0.2, output: 6.0 }
}

#[test]
fn the_cost_is_computed_from_the_exports_tokens() {
    let exported = json(r#"{"usage":{"input_tokens":1000000,"cached_input_tokens":200000,"output_tokens":500000}}"#);
    assert_eq!(computed_cost(Some(&exported), Some(&prices())), Some(4.64));
}

#[test]
fn a_cost_without_prices_is_unknown() {
    let exported = json(r#"{"usage":{"input_tokens":1000000,"output_tokens":500000}}"#);
    assert_eq!(computed_cost(Some(&exported), None), None);
}

#[test]
fn a_cost_without_counts_is_unknown() {
    for exported in [
        None,
        Some(json(r#"{"usage":"none"}"#)),
        Some(json(r#"{"usage":{"input_tokens":1000000}}"#)),
        Some(json(r#"{"usage":{"input_tokens":-1,"output_tokens":500000}}"#)),
        Some(json(r#"{"usage":{"input_tokens":"many","output_tokens":500000}}"#)),
    ] {
        assert_eq!(computed_cost(exported.as_ref(), Some(&prices())), None);
    }
}

#[test]
fn a_count_that_is_not_finite_is_unknown() {
    let mut usage = Object::new();
    usage.set("input_tokens", Value::Num(f64::INFINITY));
    usage.set("output_tokens", Value::Num(1.0));
    let exported = export_with_usage(Value::Obj(usage));
    assert_eq!(computed_cost(Some(&exported), Some(&prices())), None);
    assert_eq!(tokens_of(Some(&exported)), None);
}

#[test]
fn uncached_input_is_what_the_cache_did_not_serve() {
    let exported = json(r#"{"usage":{"input_tokens":1000000,"output_tokens":0}}"#);
    assert_eq!(computed_cost(Some(&exported), Some(&prices())), Some(2.0));
    let exported = json(r#"{"usage":{"input_tokens":100,"cached_input_tokens":500,"output_tokens":0}}"#);
    assert_eq!(computed_cost(Some(&exported), Some(&prices())), Some(0.0001));
}

#[test]
fn the_tokens_are_the_exports_counts() {
    let exported = json(r#"{"usage":{"input_tokens":1000000,"cached_input_tokens":200000,"output_tokens":500000}}"#);
    assert_eq!(tokens_of(Some(&exported)), Some(CriticTokens { cached: 200000.0, input: 1000000.0, output: 500000.0 }));
}

#[test]
fn tokens_without_a_cached_count_are_uncached() {
    let exported = json(r#"{"usage":{"input_tokens":10,"output_tokens":5}}"#);
    assert_eq!(tokens_of(Some(&exported)), Some(CriticTokens { cached: 0.0, input: 10.0, output: 5.0 }));
}

#[test]
fn tokens_without_counts_are_unknown() {
    for exported in [None, Some(json(r#"{"usage":{"output_tokens":5}}"#)), Some(json(r#"{"usage":{"input_tokens":10}}"#))] {
        assert_eq!(tokens_of(exported.as_ref()), None);
    }
}

#[test]
fn an_answer_is_the_messages_text_or_its_failure() {
    let answered = VibeMessage::Ok { text: "yes".to_string(), cost_usd: Some(0.5) };
    assert_eq!(answer_of(&answered), ModelAnswer::Answered { text: "yes".to_string(), cost_usd: Some(0.5) });
    let failed_message = VibeMessage::Failed {
        failure: "timeout".to_string(),
        detail: "killed".to_string(),
        cost_usd: None,
    };
    assert_eq!(
        answer_of(&failed_message),
        ModelAnswer::Failed { failure: "timeout".to_string(), detail: "killed".to_string(), cost_usd: None }
    );
}

#[test]
fn a_token_cost_prices_the_cached_part_at_the_cached_price() {
    let usage = TokenUsage { cached_input: 200000.0, input: 1000000.0, output: 500000.0 };
    assert_eq!(crate::pricing::token_cost(&usage, &prices()), 4.64);
}

/// A file system of the files a test names, and nothing else; `broken` is a path whose look fails, `ghosts` paths that
/// are there but cannot be read.
#[derive(Clone, Default)]
pub struct FakeFiles {
    files: Arc<Mutex<HashMap<String, String>>>,
    broken: Arc<Mutex<Option<String>>>,
    ghosts: Arc<Mutex<Vec<String>>>,
}

impl FakeFiles {
    pub fn put(&self, path: &str, text: &str) {
        if let Ok(mut files) = self.files.lock() {
            files.insert(path.to_string(), text.to_string());
        }
    }

    fn break_on(&self, path: &str) {
        if let Ok(mut broken) = self.broken.lock() {
            *broken = Some(path.to_string());
        }
    }

    fn ghost(&self, path: &str) {
        if let Ok(mut ghosts) = self.ghosts.lock() {
            ghosts.push(path.to_string());
        }
    }

    pub fn has(&self, path: &str) -> bool {
        self.files.lock().map(|files| files.contains_key(path)).unwrap_or(false)
    }

    fn is_ghost(&self, path: &str) -> bool {
        self.ghosts.lock().map(|ghosts| ghosts.iter().any(|ghost| ghost == path)).unwrap_or(false)
    }
}

/// The platform's words for a file that is not there.
fn missing(path: &str) -> FsError {
    FsError { code: "ENOENT".to_string(), message: format!("ENOENT: no such file or directory, open '{path}'") }
}

impl Files for FakeFiles {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        if self.is_ghost(path) {
            return Err(missing(path));
        }
        self.files.lock().ok().and_then(|files| files.get(path).cloned()).ok_or_else(|| missing(path))
    }

    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.write_text(path, content)
    }

    fn mkdir_all(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        if self.has(path) {
            return Ok(false);
        }
        self.put(path, "");
        Ok(true)
    }

    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        if let Ok(mut files) = self.files.lock() {
            files.remove(path);
        }
        Ok(())
    }

    fn modified_ms(&self, _path: &str) -> Option<i64> {
        None
    }

    fn exists(&self, path: &str) -> Result<bool, FsError> {
        if self.broken.lock().ok().and_then(|broken| broken.clone()).as_deref() == Some(path) {
            return Err(missing(path));
        }
        Ok(self.has(path) || self.is_ghost(path))
    }

    fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
        let existing = self.files.lock().ok().and_then(|files| files.get(path).cloned()).unwrap_or_default();
        self.put(path, &format!("{existing}{content}"));
        Ok(())
    }

    fn stamp(&self, _path: &str) -> Option<FileStamp> {
        None
    }

    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.write_text(path, content)
    }

    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.put(path, content);
        Ok(())
    }

    fn temp_dir(&self, _parent: &str, _prefix: &str) -> Result<String, FsError> {
        Err(missing("temp"))
    }

    fn remove(&self, path: &str) -> Result<(), FsError> {
        self.remove_file(path)
    }
}

/// A process that answers every launch with one canned result, or refuses to start, and records what it was given.
#[derive(Clone)]
pub struct FakeProcess {
    result: ProcessResult,
    spawn_error: Option<String>,
    commands: Arc<Mutex<Vec<ProcessCommand>>>,
}

impl FakeProcess {
    pub fn answering(result: ProcessResult) -> FakeProcess {
        FakeProcess { result, spawn_error: None, commands: Arc::new(Mutex::new(Vec::new())) }
    }

    pub fn refusing(message: &str) -> FakeProcess {
        FakeProcess { result: exit(0), spawn_error: Some(message.to_string()), commands: Arc::new(Mutex::new(Vec::new())) }
    }

    pub fn commands(&self) -> Vec<ProcessCommand> {
        self.commands.lock().map(|commands| commands.clone()).unwrap_or_default()
    }
}

impl Process for FakeProcess {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Ok(self.result.clone())
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        if let Ok(mut commands) = self.commands.lock() {
            commands.push(command.clone());
        }
        match &self.spawn_error {
            Some(message) => Err(message.clone()),
            None => Ok(self.result.clone()),
        }
    }
}

/// A finished command that printed `stdout` and exited `code`.
pub fn exit(code: i32) -> ProcessResult {
    ProcessResult { stdout: String::new(), stderr: String::new(), exit_code: code, timed_out: false }
}

/// A runtime whose files and process are the test's own, in the runner's hands and the gateway's.
pub fn runtime_with(files: impl Fn() -> Box<dyn Files>, process: impl Fn() -> Box<dyn Process>, env: &[(&str, &str)]) -> Runtime {
    let env: HashMap<String, String> = env.iter().map(|(key, value)| ((*key).to_string(), (*value).to_string())).collect();
    let mut gateway = model_gateway::default_runtime();
    gateway.files = files();
    gateway.process = process();
    gateway.env = env.clone();
    Runtime {
        clock: Box::new(SystemClock),
        files: files(),
        dirs: Box::new(SystemDirs),
        env,
        assets_root: String::new(),
        git: Box::new(SystemGit { env: HashMap::new() }),
        gh: Box::new(crate::gh::SystemGh { env: HashMap::new() }),
        gateway,
        stdin: Box::new(String::new),
        search: None,
    }
}

/// A runtime over the fakes.
pub fn fake_runtime(files: &FakeFiles, process: &FakeProcess, env: &[(&str, &str)]) -> Runtime {
    runtime_with(|| -> Box<dyn Files> { Box::new(files.clone()) }, || -> Box<dyn Process> { Box::new(process.clone()) }, env)
}

/// The Vibe critic client routing names.
pub fn vibe_config() -> VibeCriticClient {
    VibeCriticClient {
        binary: "vibe".to_string(),
        agent: "critic-mistral-medium".to_string(),
        model: "mistral-medium-3".to_string(),
        family: "mistral".to_string(),
        prices: None,
        gateway: None,
    }
}

/// One question put to the model in a packet of its own.
fn question() -> ModelQuestion {
    ModelQuestion {
        work_dir: "/packet".to_string(),
        prompt_path: "/packet/prompt.md".to_string(),
        output_dir: "/packet".to_string(),
        max_price_usd: 1.0,
        time_limit_s: 120.0,
    }
}

/// The question's run, or the error it failed with.
fn asked(runtime: &Runtime, question: &ModelQuestion, config: &VibeCriticClient) -> Result<ModelAnswerReport, CritiqueError> {
    vibe_ask(runtime, question, config)
}

/// A file system holding the question's prompt and the export a run wrote.
fn asked_files(exported: &str) -> FakeFiles {
    let files = FakeFiles::default();
    files.put("/packet/prompt.md", "is this risky?");
    files.put("/packet/export/export.json", exported);
    files
}

#[test]
fn a_question_is_answered_by_the_launch() {
    let files = asked_files(ISOLATED);
    let process = FakeProcess::answering(ProcessResult { stdout: JOURNAL.to_string(), ..exit(0) });
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let report = asked(&runtime, &question(), &vibe_config()).unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(report.answer, ModelAnswer::Answered { text: "the answer".to_string(), cost_usd: None });
    assert_eq!(report.exit_code, 0);
    assert_eq!(report.stdout, JOURNAL);
    assert_eq!(report.stderr, "");
    assert_eq!(report.charged_usd, 1.0);
    assert_eq!(report.tokens, None);
    assert_eq!(report.vibe_cost_usd, None);
}

#[test]
fn the_launch_is_the_gateways_own_command() {
    let files = asked_files(ISOLATED);
    let process = FakeProcess::answering(ProcessResult { stdout: JOURNAL.to_string(), ..exit(0) });
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    assert!(asked(&runtime, &question(), &vibe_config()).is_ok());
    let commands = process.commands();
    assert_eq!(commands.len(), 1);
    let command = &commands[0];
    assert_eq!(command.command, "env");
    assert_eq!(command.cwd.as_deref(), Some("/packet"));
    assert_eq!(command.timeout_ms, Some(180_000));
    assert_eq!(command.args.first().map(String::as_str), Some("-u"));
    assert_eq!(command.args.get(4).map(String::as_str), Some("vibe"));
    for arg in ["--prompt-file", "/packet/prompt.md", "--agent", "critic-mistral-medium", "--enabled-tools", "read_file", "--time-limit", "120", "--max-price", "1", "--output-dir", "/packet/export"] {
        assert!(command.args.iter().any(|given| given == arg), "{arg} is not in {:?}", command.args);
    }
}

#[test]
fn a_pinned_gateway_client_runs_the_launch() {
    let files = asked_files(ISOLATED);
    let process = FakeProcess::answering(ProcessResult { stdout: JOURNAL.to_string(), ..exit(0) });
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let mut config = vibe_config();
    config.gateway = Some(GatewayModel { client: "vibe".to_string(), model: "mistral-large-3".to_string() });
    assert!(asked(&runtime, &question(), &config).is_ok());
    assert_eq!(process.commands().len(), 1);
}

#[test]
fn a_gateway_client_the_gateway_does_not_run_is_refused() {
    let files = asked_files(ISOLATED);
    let process = FakeProcess::answering(exit(0));
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let mut config = vibe_config();
    config.gateway = Some(GatewayModel { client: "gemini".to_string(), model: "gemini-3".to_string() });
    assert_eq!(
        asked(&runtime, &question(), &config),
        Err(CritiqueError::new(
            "not-supported-yet",
            "only vibe, agy, claude, opencode, hermes, codex can run a delegation, not gemini"
        ))
    );
    assert!(process.commands().is_empty());
}

#[test]
fn a_failed_exit_is_a_failed_answer() {
    let files = asked_files(ISOLATED);
    let process = FakeProcess::answering(ProcessResult { stderr: "bad config\n".to_string(), ..exit(1) });
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let report = asked(&runtime, &question(), &vibe_config()).unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(
        report.answer,
        ModelAnswer::Failed { failure: "config-error".to_string(), detail: "vibe exited 1: bad config".to_string(), cost_usd: None }
    );
    assert_eq!(report.exit_code, 1);
}

#[test]
fn a_timed_out_launch_charges_the_cap() {
    let files = asked_files(ISOLATED);
    let process = FakeProcess::answering(ProcessResult { timed_out: true, ..exit(124) });
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let report = asked(&runtime, &question(), &vibe_config()).unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(
        report.answer,
        ModelAnswer::Failed {
            failure: "timeout".to_string(),
            detail: "vibe did not return within 180s, so the outer timer killed it".to_string(),
            cost_usd: None,
        }
    );
    assert_eq!(report.charged_usd, 1.0);
}

#[test]
fn a_priced_export_is_what_the_run_cost() {
    let files = asked_files(r#"{"config":{"enable_mcp":false,"enabled_tools":["read_file"],"include_project_context":false,"outside_workdir":"deny"},"cost_usd":0.25,"usage":{"input_tokens":10,"output_tokens":5}}"#);
    let process = FakeProcess::answering(ProcessResult { stdout: JOURNAL.to_string(), ..exit(0) });
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let report = asked(&runtime, &question(), &vibe_config()).unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(report.vibe_cost_usd, Some(0.25));
    assert_eq!(report.charged_usd, 0.25);
    assert_eq!(report.tokens, Some(CriticTokens { cached: 0.0, input: 10.0, output: 5.0 }));
}

#[test]
fn a_cost_that_is_not_a_number_is_no_cost() {
    let files = asked_files(r#"{"config":{"enable_mcp":false,"enabled_tools":["read_file"],"include_project_context":false,"outside_workdir":"deny"},"cost_usd":"0.25"}"#);
    let process = FakeProcess::answering(ProcessResult { stdout: JOURNAL.to_string(), ..exit(0) });
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let report = asked(&runtime, &question(), &vibe_config()).unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(report.vibe_cost_usd, None);
    assert_eq!(report.charged_usd, 1.0);
}

#[test]
fn a_prompt_that_cannot_be_read_is_the_platforms_words() {
    let files = FakeFiles::default();
    let process = FakeProcess::answering(exit(0));
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    assert_eq!(
        asked(&runtime, &question(), &vibe_config()),
        Err(CritiqueError::unexpected("ENOENT: no such file or directory, open '/packet/prompt.md'"))
    );
}

#[test]
fn a_gateway_refusal_is_the_runners_error() {
    let files = asked_files(ISOLATED);
    files.break_on("/packet/export/export.json");
    let process = FakeProcess::answering(exit(0));
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    assert_eq!(
        asked(&runtime, &question(), &vibe_config()),
        Err(CritiqueError::unexpected("ENOENT: no such file or directory, open '/packet/export/export.json'"))
    );
}

#[test]
fn a_program_that_will_not_start_is_unexpected() {
    let files = asked_files(ISOLATED);
    let process = FakeProcess::refusing("spawn vibe ENOENT");
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    assert_eq!(asked(&runtime, &question(), &vibe_config()), Err(CritiqueError::unexpected("spawn vibe ENOENT")));
}

/// A home that is signed in the way setup checks it, with the critic agent installed.
pub fn ready_files() -> FakeFiles {
    let files = FakeFiles::default();
    files.put("/bin/vibe", "");
    files.put("/home/owner/.vibe/.env", "MISTRAL_API_KEY=sk-test\n");
    files.put("/home/owner/.vibe/agents/critic-mistral-medium.toml", "");
    files
}

/// A process whose keyring holds nothing, so the home's `.env` decides.
pub fn empty_keyring() -> FakeProcess {
    FakeProcess::answering(exit(1))
}

#[test]
fn a_signed_in_home_with_its_agent_is_ready() {
    let files = ready_files();
    let runtime = fake_runtime(&files, &empty_keyring(), &[("PATH", "/bin"), ("HOME", "/home/owner")]);
    assert_eq!(vibe_readiness(&runtime, &vibe_config()), Ok(ClientReadiness { ready: true, detail: None }));
}

#[test]
fn a_binary_that_is_not_on_path_is_not_ready() {
    let files = FakeFiles::default();
    let runtime = fake_runtime(&files, &empty_keyring(), &[("PATH", "/bin"), ("HOME", "/home/owner")]);
    assert_eq!(
        vibe_readiness(&runtime, &vibe_config()),
        Ok(ClientReadiness { ready: false, detail: Some("vibe: vibe is not on PATH".to_string()) })
    );
}

#[test]
fn a_path_that_is_missing_or_empty_finds_no_binary() {
    for env in [vec![("HOME", "/home/owner")], vec![("PATH", "::"), ("HOME", "/home/owner")]] {
        let files = ready_files();
        let runtime = fake_runtime(&files, &empty_keyring(), &env);
        assert_eq!(
            vibe_readiness(&runtime, &vibe_config()),
            Ok(ClientReadiness { ready: false, detail: Some("vibe: vibe is not on PATH".to_string()) })
        );
    }
}

#[test]
fn a_path_folder_that_cannot_be_looked_in_holds_no_binary() {
    let files = ready_files();
    files.break_on("/bin/vibe");
    let runtime = fake_runtime(&files, &empty_keyring(), &[("PATH", "/bin"), ("HOME", "/home/owner")]);
    assert_eq!(
        vibe_readiness(&runtime, &vibe_config()),
        Ok(ClientReadiness { ready: false, detail: Some("vibe: vibe is not on PATH".to_string()) })
    );
}

#[test]
fn an_absolute_binary_is_looked_for_where_it_stands() {
    let files = ready_files();
    files.put("/opt/vibe/bin/vibe", "");
    let runtime = fake_runtime(&files, &empty_keyring(), &[("PATH", "/bin"), ("HOME", "/home/owner")]);
    let mut config = vibe_config();
    config.binary = "/opt/vibe/bin/vibe".to_string();
    assert_eq!(vibe_readiness(&runtime, &config), Ok(ClientReadiness { ready: true, detail: None }));
    config.binary = "/opt/other/vibe".to_string();
    assert_eq!(
        vibe_readiness(&runtime, &config),
        Ok(ClientReadiness { ready: false, detail: Some("vibe: /opt/other/vibe is not on PATH".to_string()) })
    );
}

#[test]
fn a_home_that_is_not_signed_in_is_not_ready() {
    let files = FakeFiles::default();
    files.put("/bin/vibe", "");
    let runtime = fake_runtime(&files, &empty_keyring(), &[("PATH", "/bin"), ("HOME", "/home/owner")]);
    assert_eq!(
        vibe_readiness(&runtime, &vibe_config()),
        Ok(ClientReadiness { ready: false, detail: Some(SIGN_IN.to_string()) })
    );
}

#[test]
fn a_missing_critic_agent_is_not_ready() {
    let files = FakeFiles::default();
    files.put("/bin/vibe", "");
    files.put("/home/owner/.vibe/.env", "MISTRAL_API_KEY=sk-test\n");
    let runtime = fake_runtime(&files, &empty_keyring(), &[("PATH", "/bin"), ("HOME", "/home/owner")]);
    let detail = "vibe: the critic agent critic-mistral-medium is not installed in ~/.vibe/agents; re-run setup".to_string();
    assert_eq!(vibe_readiness(&runtime, &vibe_config()), Ok(ClientReadiness { ready: false, detail: Some(detail.clone()) }));
    files.break_on("/home/owner/.vibe/agents/critic-mistral-medium.toml");
    assert_eq!(vibe_readiness(&runtime, &vibe_config()), Ok(ClientReadiness { ready: false, detail: Some(detail) }));
}

#[test]
fn a_home_whose_key_cannot_be_read_is_an_error() {
    let files = FakeFiles::default();
    files.put("/bin/vibe", "");
    files.ghost("/home/owner/.vibe/.env");
    let runtime = fake_runtime(&files, &empty_keyring(), &[("PATH", "/bin"), ("HOME", "/home/owner")]);
    assert_eq!(
        vibe_sign_in(&runtime, &vibe_config()),
        Err(CritiqueError::unexpected("ENOENT: no such file or directory, open '/home/owner/.vibe/.env'"))
    );
}

#[test]
fn signing_in_ignores_the_agents() {
    let files = FakeFiles::default();
    files.put("/bin/vibe", "");
    files.put("/home/owner/.vibe/.env", "MISTRAL_API_KEY=sk-test\n");
    let runtime = fake_runtime(&files, &empty_keyring(), &[("PATH", "/bin"), ("HOME", "/home/owner")]);
    assert_eq!(vibe_sign_in(&runtime, &vibe_config()), Ok(ClientReadiness { ready: true, detail: None }));
}

#[test]
fn a_missing_home_is_an_error() {
    let files = ready_files();
    let runtime = fake_runtime(&files, &empty_keyring(), &[("PATH", "/bin")]);
    assert_eq!(
        vibe_readiness(&runtime, &vibe_config()),
        Err(CritiqueError::unexpected("required environment variable is missing: HOME"))
    );
    assert_eq!(
        vibe_sign_in(&runtime, &vibe_config()),
        Err(CritiqueError::unexpected("required environment variable is missing: HOME"))
    );
}

/// A work client of a kind no machine wires yet.
pub fn agy_config() -> AgyCriticClient {
    AgyCriticClient { binary: "agy".to_string(), model: "gemini-3".to_string(), family: "google".to_string(), gateway: None }
}

pub fn codex_config() -> CodexCriticClient {
    CodexCriticClient { binary: "codex".to_string(), model: "gpt-5".to_string(), family: "openai".to_string(), gateway: None }
}

pub fn claude_config() -> ClaudeCriticClient {
    ClaudeCriticClient { binary: "claude".to_string(), model: "opus".to_string(), family: "anthropic".to_string(), gateway: None }
}

/// A fresh directory under the system's temporary one, named for this test.
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("critique-vibe-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A `vibe` that writes the export the launcher reads and prints the journal a critic's run leaves.
const FAKE_VIBE: &str = "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --output-dir) out=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nprintf '%s' '{\"config\":{\"enable_mcp\":false,\"enabled_tools\":[\"read_file\"],\"include_project_context\":false,\"outside_workdir\":\"deny\"},\"cost_usd\":0.25}' > \"$out/export.json\"\nprintf '%s' '[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"text\":\"yes\"}]}]'\n";

#[test]
fn a_real_vibe_launch_reads_its_journal_and_its_export() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("launch");
    let bin = dir.join("bin");
    let packet = dir.join("packet");
    let home = dir.join("home");
    for path in [&bin, &packet, &home, &packet.join("export")] {
        assert!(std::fs::create_dir_all(path).is_ok());
    }
    let script = bin.join("vibe");
    assert!(std::fs::write(&script, FAKE_VIBE).is_ok());
    assert!(std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).is_ok());
    assert!(std::fs::write(packet.join("prompt.md"), "is this risky?").is_ok());
    let path = format!("{}:/usr/bin:/bin", bin.to_string_lossy());
    let home_text = home.to_string_lossy().into_owned();
    let env = [("PATH", path.as_str()), ("HOME", home_text.as_str())];
    let cwd = dir.to_string_lossy().into_owned();
    let child_env: HashMap<String, String> = env.iter().map(|(key, value)| ((*key).to_string(), (*value).to_string())).collect();
    let runtime = runtime_with(
        || -> Box<dyn Files> { Box::new(SystemFiles { cwd: cwd.clone() }) },
        || -> Box<dyn Process> { Box::new(SystemProcess { env: child_env.clone() }) },
        &env,
    );
    let question = ModelQuestion {
        work_dir: packet.to_string_lossy().into_owned(),
        prompt_path: packet.join("prompt.md").to_string_lossy().into_owned(),
        output_dir: packet.to_string_lossy().into_owned(),
        max_price_usd: 1.0,
        time_limit_s: 120.0,
    };
    let report = asked(&runtime, &question, &vibe_config()).unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(report.answer, ModelAnswer::Answered { text: "yes".to_string(), cost_usd: Some(0.25) });
    assert_eq!(report.exit_code, 0);
    assert_eq!(report.vibe_cost_usd, Some(0.25));
    assert_eq!(report.charged_usd, 0.25);
    assert_eq!(report.tokens, None);
    let _ = std::fs::remove_dir_all(&dir);
}
