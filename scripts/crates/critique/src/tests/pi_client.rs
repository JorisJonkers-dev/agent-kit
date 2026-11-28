//! The Pi critic client's readers, at the edges the goldens do not reach. The expected words are the node runner's,
//! from `adapters/pi`.

use crate::clients::CriticOutcome;
use crate::pi_client::{auth_readiness, classify_pi_run, outside_calls, outside_packet, read_pi_stream, PiToolCall};
use model_gateway::json::parse;
use crate::routing::PiCriticClient;
use model_gateway::ports::ProcessResult;

fn config() -> PiCriticClient {
    PiCriticClient { binary: "pi".to_string(), provider: "litellm-vibe-study".to_string(), model: "glm-5-3".to_string(), family: "zhipu".to_string(), data_scope: "any".to_string(), free: false, tier: None }
}

fn result(stdout: &str, timed_out: bool) -> ProcessResult {
    ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out }
}

fn failure(outcome: CriticOutcome) -> (String, String) {
    match outcome {
        CriticOutcome::Failed { failure, detail, .. } => (failure, detail),
        CriticOutcome::Ok { .. } => panic!("not a failure"),
    }
}

#[test]
fn an_auth_check_that_could_not_start_or_did_not_answer_says_why() {
    let spawn = auth_readiness(&Err("spawn pi EACCES".to_string()), &config());
    assert_eq!(spawn.detail.as_deref(), Some("pi: auth check for litellm-vibe-study failed: spawn pi EACCES"));
    let slow = auth_readiness(&Ok(result("", true)), &config());
    assert_eq!(slow.detail.as_deref(), Some("pi: auth check for litellm-vibe-study failed: no answer within 20s"));
}

#[test]
fn a_stream_counts_no_tokens_a_message_does_not_report_and_reads_only_text_parts() {
    let stdout = [
        r#"{"type":"message_end","message":{"role":"assistant","content":"plain","usage":{"input":1}}}"#,
        r#"{"type":"message_end","message":{"role":"user","content":[]}}"#,
    ]
    .join("\n");
    let stream = read_pi_stream(&stdout);
    assert_eq!((stream.answer.as_deref(), stream.cost_usd, stream.tokens), (Some(""), None, None));
}

#[test]
fn a_run_the_timer_killed_printed_nothing_or_ended_in_no_assistant_message_is_no_review() {
    let classify = |stdout: &str, timed_out: bool| failure(classify_pi_run(&result(stdout, timed_out), &read_pi_stream(stdout), "/p", 600.0, &allowed_tools(&READ_ONLY_TOOLSET)));
    assert_eq!(classify("", true), ("timeout".to_string(), "pi did not return within 600s, so the outer timer killed it".to_string()));
    assert_eq!(classify(" \n", false), ("empty-output".to_string(), "pi exited 0 with empty stdout".to_string()));
    assert_eq!(classify(r#"{"type":"agent_start"}"#, false), ("unparseable-output".to_string(), "stdout is not a Pi event stream with an assistant message".to_string()));
}

#[test]
fn a_path_is_resolved_the_way_pi_resolves_it_before_it_is_judged() {
    let root = "/runs/c1/packet";
    for inside in ["tree/../tree/a.ts", "./a/./b", "@tree/a.ts", "/runs/c1/packet", "/runs/c1/packet/tree/b.ts"] {
        assert!(!outside_packet(inside, root), "{inside}");
    }
    for outside in ["..", "a/../../b", "@/etc/passwd", "~", "/runs/c1/packet/../secret", "/runs/c1/packetx", "/runs", "/../runs/c1/packet/a"] {
        assert!(outside_packet(outside, root), "{outside}");
    }
}

#[test]
fn every_call_that_left_the_packet_is_named_once() {
    let call = |tool: &str, args: &str| PiToolCall { tool: tool.to_string(), args: parse(args).ok().and_then(|value| value.as_object().cloned()) };
    let calls = [call("read", r#"{"path":"~/.ssh/id"}"#), call("read", r#"{"path":"~/.ssh/id"}"#), call("grep", r#"{"pattern":"../x"}"#), call("ls", "5")];
    assert_eq!(outside_calls(&calls, "/p"), vec!["read ~/.ssh/id".to_string()]);
}

// The sandbox: the home's route, the key and the readiness check, as the node runner's `adapters/pi` reads them.

use super::vibe_client::{runtime_with, FakeFiles};
use crate::critic_tools::{CriticToolset, SearchServerLaunch, READ_ONLY_TOOLSET};
use crate::packet_search::{default_search_limits, SearchToolName};
use crate::pi_client::{allowed_tools, mcp_servers, SearchAccess};
use crate::clients::{CriticLaunch, CriticReport};
use crate::pi_client::{
    confined_command, grants, home_agent_dir, key, pi_command, pi_readiness, pi_review, prepare, route, route_of, sandbox_readiness, url_port, PiRoute,
};
use crate::roster::RosterCritic;
use crate::routing::CriticClient;
use model_gateway::json::Value;
use model_gateway::ports::{Files, Process, ProcessCommand};
use std::sync::{Arc, Mutex};

const HOME: &str = "/h";
const MODELS: &str = "/h/.pi/agent/models.json";

/// A process that answers each command line by what it starts with, and keeps every one it was given.
#[derive(Clone, Default)]
struct Scripted {
    answers: Arc<Mutex<Vec<(String, Result<ProcessResult, String>)>>>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Scripted {
    fn on(self, prefix: &str, answer: Result<ProcessResult, String>) -> Scripted {
        self.answers.lock().unwrap().push((prefix.to_string(), answer));
        self
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    fn answer(&self, line: String) -> Result<ProcessResult, String> {
        self.seen.lock().unwrap().push(line.clone());
        let answers = self.answers.lock().unwrap();
        answers.iter().find(|(prefix, _)| line.starts_with(prefix.as_str())).map_or(Ok(ok("")), |(_, answer)| answer.clone())
    }
}

impl Process for Scripted {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, command: &str, args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.answer(format!("{command} {}", args.join(" ")))
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.answer(format!("{} {}", command.command, command.args.join(" ")))
    }
}

fn ok(stdout: &str) -> ProcessResult {
    ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out: false }
}

fn failing(code: i32, stderr: &str, stdout: &str) -> ProcessResult {
    ProcessResult { stdout: stdout.to_string(), stderr: stderr.to_string(), exit_code: code, timed_out: false }
}

/// The home's providers as setup writes them, with `api_key` as the vibe-study route's key.
fn home_models(api_key: &str) -> String {
    format!(
        r#"{{"providers":{{"litellm-vibe-study":{{"api":"openai-completions","apiKey":"{api_key}","baseUrl":"https://litellm.example.test/v1","models":[{{"id":"other"}},{{"cost":{{"input":0.5}},"id":"glm-5-3"}}]}}}}}}"#
    )
}

const SECRET: &str = "!/bin/agent-kit-secret get LITELLM_VIBE_STUDY_KEY";

/// A machine with pi on PATH, ready, its launches handed to the process as plan commands, and the home's providers as given.
fn machine(models: Option<&str>, process: Scripted, env: &[(&str, &str)]) -> (FakeFiles, crate::runtime::Runtime) {
    let files = FakeFiles::default();
    files.put("/bin/pi", "");
    if let Some(models) = models {
        files.put(MODELS, models);
    }
    let mut all = vec![("HOME", HOME), ("PATH", "/bin"), ("AGENT_KIT_SANDBOX", "host")];
    all.extend_from_slice(env);
    let process = process
        .on("pi auth", Ok(ok(r#"{"status":"ready"}"#)))
        .on("pi --offline", Ok(ok("litellm-vibe-study glm-5-3 128K")))
        .on("/bin/agent-kit-secret get", Ok(ok("sk-route\n")));
    let fake = files.clone();
    let runtime = runtime_with(move || -> Box<dyn Files> { Box::new(fake.clone()) }, move || -> Box<dyn Process> { Box::new(process.clone()) }, &all);
    (files, runtime)
}

fn route_for(port: u16) -> PiRoute {
    PiRoute { base_url: "u".to_string(), api: "a".to_string(), api_key: "k".to_string(), model: Value::Null, port }
}

#[test]
fn a_url_port_is_the_one_after_the_host_or_the_schemes_own() {
    let ports: Vec<Option<u16>> = ["https://p", "http://p/v1", "http://p:8080", "https://p:0/", "https://p:x", "https://p:99999", "https://p:", "p"].iter().map(|url| url_port(url)).collect();
    assert_eq!(ports, vec![Some(443), Some(80), Some(8080), None, None, None, None, None]);
}

#[test]
fn the_route_is_read_from_the_homes_provider_or_says_why_not() {
    let at = "/h/models.json";
    let provider = |text: &str| parse(text).unwrap().as_object().cloned().unwrap();
    let full = provider(r#"{"api":"a","apiKey":"k","baseUrl":"https://p.test:8443/v1","models":[{"id":"other"},"x"]}"#);
    assert_eq!(route_of(Some(&full), &config(), at), Ok(PiRoute { base_url: "https://p.test:8443/v1".to_string(), api: "a".to_string(), api_key: "k".to_string(), model: parse(r#"{"id":"glm-5-3"}"#).unwrap(), port: 8443 }));
    let unlisted = provider(r#"{"api":"a","apiKey":"k","baseUrl":"https://p"}"#);
    assert_eq!(route_of(Some(&unlisted), &config(), at).unwrap().model, parse(r#"{"id":"glm-5-3"}"#).unwrap());
    assert!(route_of(None, &config(), at).unwrap_err().contains("/h/models.json has no litellm-vibe-study provider"));
    assert!(route_of(Some(&provider(r#"{"api":"a","apiKey":5,"baseUrl":"https://p"}"#)), &config(), at).unwrap_err().contains("lacks a baseUrl, api or apiKey"));
    assert!(route_of(Some(&provider(r#"{"api":"a","apiKey":"k","baseUrl":"ftp://p"}"#)), &config(), at).unwrap_err().contains("is not an http or https URL"));
}

#[test]
fn the_sandbox_check_is_read_and_the_plan_command_line_built_with_or_without_a_path() {
    assert!(sandbox_readiness(&Ok(())).ready);
    let refused = sandbox_readiness(&Err(model_gateway::error::GatewayError::new("sandbox-unavailable", "sbx is not usable (spawn sbx ENOENT)")));
    assert!(!refused.ready && refused.detail.unwrap().ends_with("none can start here: sbx is not usable (spawn sbx ENOENT)"));
    let roster = RosterCritic { client: "c".to_string(), role: "r".to_string(), lenses: vec![], max_price_usd: 1.0, time_limit_s: 1.0, critic_id: "c1".to_string(), client_config: CriticClient::Pi(config()), fallbacks: vec![] };
    let launch = CriticLaunch { critic: &roster, packet_dir: "/p".to_string(), prompt_path: "/c/prompt.md".to_string(), critic_dir: "/c".to_string(), schema_path: String::new(), toolset: READ_ONLY_TOOLSET };
    let pi = || pi_command(&config(), &launch, "prompt".to_string());
    let env = |command: ProcessCommand| command.env.get("PATH").cloned().unwrap_or_default();
    assert_eq!(env(confined_command(vec![], pi(), None, "/a", None)), "");
    assert_eq!(env(confined_command(vec![], pi(), Some(""), "/a", Some("/b"))), "/b");
    let full = confined_command(vec!["--read".to_string(), "/p".to_string()], pi(), Some("/usr/bin"), "/a", Some("/b"));
    assert_eq!((full.command.as_str(), &full.args[..4], full.cwd.as_deref(), full.stdin.as_deref()), ("sbx-plan", &["grants", "--read", "/p", "--"].map(String::from)[..], Some("/p"), Some("prompt")));
    assert_eq!((env(full.clone()), full.env.get("PI_CODING_AGENT_DIR").cloned(), full.env.get("TMPDIR").cloned()), ("/b:/usr/bin".to_string(), Some("/a".to_string()), Some("/a".to_string())));
}

#[test]
fn the_home_pi_directory_follows_its_variable() {
    let (_, plain) = machine(None, Scripted::default(), &[]);
    assert_eq!(home_agent_dir(&plain), "/h/.pi/agent".to_string());
    let (_, named) = machine(None, Scripted::default(), &[("PI_CODING_AGENT_DIR", "/own")]);
    assert_eq!(home_agent_dir(&named), "/own".to_string());
}

#[test]
fn the_grants_add_the_homes_pi_bin_and_a_launchs_packet_and_agent_directory() {
    let (files, runtime) = machine(None, Scripted::default(), &[]);
    assert_eq!(grants(&runtime, &config(), &route_for(443), None, None), ["--run-from", "pi", "--net-connect", "443", "--env", "PI_CODING_AGENT_DIR"].map(String::from));
    files.put("/h/.pi/agent/bin", "");
    assert_eq!(
        grants(&runtime, &config(), &route_for(8443), Some(("/p", "/a")), None),
        ["--read", "/p", "--run-from", "pi", "--run", "/h/.pi/agent/bin", "--write", "/a", "--net-connect", "8443", "--env", "PI_CODING_AGENT_DIR"].map(String::from)
    );
}

#[test]
fn the_key_is_a_literal_or_read_from_the_secret_store_by_name_and_a_failed_read_shows_no_part_of_it() {
    let (_, runtime) = machine(None, Scripted::default(), &[]);
    assert_eq!(key(&runtime, &config(), "sk-literal"), Ok("sk-literal".to_string()));
    assert_eq!(key(&runtime, &config(), SECRET), Ok("sk-route".to_string()));
    assert!(key(&runtime, &config(), "!cat /run/key | tr -d x").unwrap_err().contains("a shell command the runner does not run"));
    for (answer, said) in [
        (Ok(failing(1, "no such secret\n", "sk-leak")), "(LITELLM_VIBE_STUDY_KEY): no such secret"),
        (Ok(failing(0, "", " \n")), "(LITELLM_VIBE_STUDY_KEY): exit 0"),
        (Ok(ProcessResult { timed_out: true, ..ok("sk-late") }), "(LITELLM_VIBE_STUDY_KEY): exit 0"),
        (Err("spawn agent-kit-secret ENOENT".to_string()), "spawn agent-kit-secret ENOENT"),
    ] {
        let (_, runtime) = machine(None, Scripted::default().on("/bin/agent-kit-secret", answer), &[]);
        let message = key(&runtime, &config(), SECRET).unwrap_err();
        assert!(message.ends_with(said) && !message.contains("sk-"), "{message}");
    }
}

#[test]
fn readiness_needs_the_route_and_a_sandbox_that_can_start() {
    let ready = |models: Option<&str>, process: Scripted, env: &[(&str, &str)]| {
        let (_, runtime) = machine(models, process, env);
        pi_readiness(&runtime, &config()).unwrap()
    };
    let models = home_models(SECRET);
    assert!(ready(Some(&models), Scripted::default(), &[]).ready);
    assert!(ready(None, Scripted::default(), &[]).detail.unwrap().contains("has no litellm-vibe-study provider"));
    assert!(ready(Some("not json"), Scripted::default(), &[]).detail.unwrap().contains("has no litellm-vibe-study provider"));
    // With sbx selected, readiness is `sbx version`, and a machine without it says how to get it.
    let sbx = [("AGENT_KIT_SANDBOX", "sbx")];
    let checked = Scripted::default().on("sbx secret ls", Ok(ok(r#"{"secrets":[],"custom_secrets":[],"shadowed_services":[],"env_only_count":0}"#)));
    assert!(ready(Some(&models), checked.clone(), &sbx).ready);
    assert!(checked.seen().contains(&"sbx version".to_string()), "{:?}", checked.seen());
    let missing = Scripted::default().on("sbx version", Err("spawn sbx ENOENT".to_string()));
    let refused = ready(Some(&models), missing, &sbx);
    assert!(!refused.ready && refused.detail.unwrap().contains("brew trust docker/tap && brew install docker/tap/sbx"));
    let unoffered = Scripted::default().on("pi --offline", Ok(ok("litellm-vibe-study other 128K")));
    assert!(ready(Some(&models), unoffered, &[]).detail.unwrap().contains("does not offer glm-5-3"));
}

#[test]
fn prepare_writes_only_the_routes_provider_with_its_key_and_says_why_it_cannot() {
    let (files, runtime) = machine(Some(&home_models(SECRET)), Scripted::default(), &[]);
    let prepared = prepare(&runtime, &config(), "/c/pi-agent", None).unwrap().unwrap();
    assert_eq!(prepared.port, 443);
    let written = parse(&files.read_text("/c/pi-agent/models.json").unwrap()).unwrap();
    let expected = parse(r#"{"providers":{"litellm-vibe-study":{"api":"openai-completions","apiKey":"sk-route","baseUrl":"https://litellm.example.test/v1","models":[{"cost":{"input":0.5},"id":"glm-5-3"}]}}}"#).unwrap();
    assert_eq!((written, files.read_text("/c/pi-agent/settings.json").unwrap()), (expected, "{}\n".to_string()));
    let (_, no_route) = machine(None, Scripted::default(), &[]);
    assert!(prepare(&no_route, &config(), "/c/pi-agent", None).unwrap().unwrap_err().contains("has no litellm-vibe-study provider"));
    let (_, bad_key) = machine(Some(&home_models("!sh -c x")), Scripted::default(), &[]);
    assert!(prepare(&bad_key, &config(), "/c/pi-agent", None).unwrap().unwrap_err().contains("a shell command"));
    assert!(route(&bad_key, &config()).is_ok());
}

#[test]
fn a_critic_that_cannot_be_sandboxed_never_starts_and_its_report_says_why() {
    let process = Scripted::default();
    let (files, runtime) = machine(None, process.clone(), &[]);
    files.put("/c/prompt.md", "prompt");
    let roster = RosterCritic { client: "c".to_string(), role: "r".to_string(), lenses: vec![], max_price_usd: 1.0, time_limit_s: 1.0, critic_id: "c1".to_string(), client_config: CriticClient::Pi(config()), fallbacks: vec![] };
    let launch = CriticLaunch { critic: &roster, packet_dir: "/c/packet".to_string(), prompt_path: "/c/prompt.md".to_string(), critic_dir: "/c".to_string(), schema_path: String::new(), toolset: READ_ONLY_TOOLSET };
    let report: CriticReport = pi_review(&runtime, &launch, &config()).unwrap();
    assert_eq!((report.exit_code, report.charged_usd, report.export_path.as_str()), (125, 0.0, "/c/result.json"));
    assert_eq!(failure(report.outcome).0, "unconfined");
    assert!(!process.seen().iter().any(|line| line.contains("sbx-plan")));
}

#[test]
fn a_launch_sbx_could_not_start_reads_as_unconfined() {
    let refused = |stderr: &str| failure(classify_pi_run(&failing(125, stderr, ""), &read_pi_stream(""), "/p", 1.0, &allowed_tools(&READ_ONLY_TOOLSET)));
    assert_eq!(refused("sbx: no microVM; nothing started\n"), ("unconfined".to_string(), "sbx could not start pi in a sandbox, so it did not start: sbx: no microVM; nothing started".to_string()));
    assert!(refused("").1.ends_with("did not start: exit 125"));
}

fn searching_launch<'a>(roster: &'a RosterCritic, server: &SearchServerLaunch) -> CriticLaunch<'a> {
    let toolset = CriticToolset::PiSearch { search: vec![SearchToolName::Grep, SearchToolName::List], limits: default_search_limits(), server: server.clone() };
    CriticLaunch { critic: roster, packet_dir: "/c/packet".to_string(), prompt_path: "/c/prompt.md".to_string(), critic_dir: "/c".to_string(), schema_path: String::new(), toolset }
}

fn roster_critic() -> RosterCritic {
    RosterCritic { client: "c".to_string(), role: "r".to_string(), lenses: vec![], max_price_usd: 1.0, time_limit_s: 1.0, critic_id: "c1".to_string(), client_config: CriticClient::Pi(config()), fallbacks: vec![] }
}

#[test]
fn a_searching_pi_launch_names_the_servers_tools_back_in_and_keeps_mcp_on_through_the_builtin_extension() {
    let roster = roster_critic();
    let server = SearchServerLaunch { command: "/opt/critique".to_string(), args: vec!["search-server".to_string(), "--root".to_string(), "/c/packet".to_string()] };
    let args = pi_command(&config(), &searching_launch(&roster, &server), "p".to_string()).args;
    let after = |flag: &str| args[args.iter().position(|arg| arg == flag).unwrap() + 1].clone();
    assert_eq!(after("--tools"), "read,grep,find,ls,mcp__packet__*");
    assert_eq!(after("-e"), "builtin:mcp");
    assert!(!args.contains(&"--no-mcp".to_string()));
    assert!(args.iter().position(|arg| arg == "--no-extensions") < args.iter().position(|arg| arg == "-e"));
    // Without the server MCP is off, as before.
    let plain = pi_command(&config(), &CriticLaunch { toolset: READ_ONLY_TOOLSET, ..searching_launch(&roster, &server) }, "p".to_string()).args;
    assert!(plain.contains(&"--no-mcp".to_string()) && !plain.contains(&"-e".to_string()));
    assert_eq!(plain[plain.iter().position(|arg| arg == "--tools").unwrap() + 1], "read,grep,find,ls");
}

#[test]
fn the_launchs_mcp_json_holds_the_search_server_alone_and_the_grants_let_pi_run_it() {
    let server = SearchServerLaunch { command: "/opt/critique".to_string(), args: vec!["/opt/server.js".to_string(), "search-server".to_string(), "--root".to_string(), "/c/packet".to_string()] };
    let written = mcp_servers(&server);
    let expected = parse(r#"{"mcpServers":{"packet":{"command":"/opt/critique","args":["/opt/server.js","search-server","--root","/c/packet"],"exposure":"direct","description":"Search the files of your working directory"}}}"#).unwrap();
    assert_eq!(written, expected);
    let (files, runtime) = machine(Some(&home_models(SECRET)), Scripted::default(), &[]);
    files.put("/opt/server.js", "");
    let grants = grants(&runtime, &config(), &route_for(443), Some(("/p", "/a")), Some(&SearchAccess::Program(&server)));
    assert_eq!(
        grants,
        ["--read", "/p", "--run-from", "pi", "--run-from", "/opt/critique", "--read", "/opt/server.js", "--write", "/a", "--net-connect", "443", "--env", "PI_CODING_AGENT_DIR"].map(String::from)
    );
    prepare(&runtime, &config(), "/c/pi-agent", Some(&SearchAccess::Program(&server))).unwrap().unwrap();
    assert_eq!(parse(&files.read_text("/c/pi-agent/mcp.json").unwrap()).unwrap(), expected);
    let (plain_files, plain) = machine(Some(&home_models(SECRET)), Scripted::default(), &[]);
    prepare(&plain, &config(), "/c/pi-agent", None).unwrap().unwrap();
    assert!(plain_files.read_text("/c/pi-agent/mcp.json").is_err());
}

#[test]
fn a_searching_critic_may_call_the_servers_tools_and_nothing_else_beyond_the_read_tools() {
    let roster = roster_critic();
    let server = SearchServerLaunch { command: "critique".to_string(), args: vec![] };
    let toolset = searching_launch(&roster, &server).toolset;
    assert_eq!(allowed_tools(&toolset), ["read", "grep", "find", "ls", "mcp__packet__grep", "mcp__packet__list"].map(String::from));
    assert_eq!(allowed_tools(&CriticToolset::Pi), ["read", "grep", "find", "ls"].map(String::from));
    let stdout = [
        r#"{"type":"tool_execution_start","toolName":"mcp__packet__grep","args":{"pattern":"x"}}"#,
        r#"{"type":"tool_execution_start","toolName":"mcp__other__grep","args":{"pattern":"x"}}"#,
        r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"x"}]}}"#,
    ]
    .join("\n");
    let stream = read_pi_stream(&stdout);
    let allowed = classify_pi_run(&result(&stdout, false), &stream, "/p", 1.0, &allowed_tools(&toolset));
    assert_eq!(failure(allowed).1, "the critic used tools beyond reading its packet: mcp__other__grep");
    let plain = classify_pi_run(&result(&stdout, false), &stream, "/p", 1.0, &allowed_tools(&CriticToolset::Pi));
    assert_eq!(failure(plain).1, "the critic used tools beyond reading its packet: mcp__packet__grep, mcp__other__grep");
}

#[test]
fn a_searching_critic_is_launched_with_the_server_granted_to_the_plan() {
    let process = Scripted::default().on("sbx-plan grants", Ok(ok(r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"x"}]}}"#)));
    let (files, runtime) = machine(Some(&home_models(SECRET)), process.clone(), &[]);
    files.put("/c/prompt.md", "prompt");
    let roster = roster_critic();
    let server = SearchServerLaunch { command: "/opt/critique".to_string(), args: vec!["search-server".to_string(), "--root".to_string(), "/c/packet".to_string()] };
    let report = pi_review(&runtime, &searching_launch(&roster, &server), &config()).unwrap();
    assert_eq!(report.exit_code, 0);
    let seen = process.seen();
    let launched = seen.iter().find(|line| line.starts_with("sbx-plan grants")).expect("a plan launch");
    assert!(launched.contains("--run-from pi --run-from /opt/critique --write /c/pi-agent --net-connect 443 --env PI_CODING_AGENT_DIR -- pi -p"), "{launched}");
    assert!(launched.contains("--tools read,grep,find,ls,mcp__packet__* --no-extensions -e builtin:mcp"), "{launched}");
}

#[test]
fn a_provider_error_that_names_a_rate_limit_or_a_quota_is_a_rate_limit_and_any_other_is_not() {
    let stopped = |message: &str| {
        let stdout = format!(r#"{{"type":"message_end","message":{{"role":"assistant","content":[],"stopReason":"error","errorMessage":"{message}"}}}}"#);
        failure(classify_pi_run(&result(&stdout, false), &read_pi_stream(&stdout), "/p", 600.0, &allowed_tools(&READ_ONLY_TOOLSET)))
    };
    assert_eq!(stopped("429 Too Many Requests").0, "rate-limited");
    assert_eq!(stopped("Rate-Limit reached").0, "rate-limited");
    assert_eq!(stopped("monthly quota used").1, "pi stopped on a rate limit: monthly quota used");
    assert_eq!(stopped("upstream exploded").0, "infrastructure-failure");
}

#[test]
fn under_sbx_a_searching_critic_calls_the_search_server_over_http_and_a_machine_without_sbx_refuses_every_pi_critic() {
    let sbx = [("AGENT_KIT_SANDBOX", "sbx")];
    let roster = roster_critic();
    let server = SearchServerLaunch { command: "/opt/critique".to_string(), args: vec![] };
    // The server is served by the runner on the host, so the critic is no longer refused for it: it goes the way every
    // sandboxed Pi critic goes, and a machine without sbx refuses it with the install command.
    let packet = std::env::temp_dir().join(format!("critique-pi-sbx-{}", std::process::id()));
    std::fs::create_dir_all(&packet).unwrap();
    let missing = Scripted::default().on("sbx version", Err("spawn sbx ENOENT".to_string()));
    let (files, runtime) = machine(Some(&home_models(SECRET)), missing.clone(), &sbx);
    files.put("/c/prompt.md", "prompt");
    let launch = CriticLaunch { packet_dir: packet.to_string_lossy().into_owned(), ..searching_launch(&roster, &server) };
    let message = format!("{:?}", pi_review(&runtime, &launch, &config()));
    assert!(message.contains("brew trust docker/tap && brew install docker/tap/sbx") && !message.contains("cannot run inside"), "{message}");
    assert!(missing.seen().iter().all(|line| line.starts_with("sbx version") || line.starts_with("/bin/agent-kit-secret")), "{:?}", missing.seen());
    // A packet the host cannot open is the reason, not a silent search-less critic.
    let (files, runtime) = machine(Some(&home_models(SECRET)), Scripted::default(), &sbx);
    files.put("/c/prompt.md", "prompt");
    let report = pi_review(&runtime, &searching_launch(&roster, &server), &config()).unwrap();
    assert!(failure(report.outcome).1.contains("could not start on the host"));
    // A critic that does not search needs no server and is handled the same way.
    let (files, runtime) = machine(Some(&home_models(SECRET)), missing, &sbx);
    files.put("/c/prompt.md", "prompt");
    let plain = CriticLaunch { critic: &roster, packet_dir: "/c/packet".to_string(), prompt_path: "/c/prompt.md".to_string(), critic_dir: "/c".to_string(), schema_path: String::new(), toolset: READ_ONLY_TOOLSET };
    assert!(format!("{:?}", pi_review(&runtime, &plain, &config())).contains("brew trust docker/tap"));
    let _ = std::fs::remove_dir_all(&packet);
}

#[test]
fn a_sandboxed_critics_mcp_json_names_the_hosts_url_and_its_grants_hold_no_server_program() {
    let url = SearchAccess::Http(4711);
    let expected = parse(r#"{"mcpServers":{"packet":{"url":"http://host.docker.internal:4711/mcp","exposure":"direct","description":"Search the files of your working directory"}}}"#).unwrap();
    assert_eq!(url.mcp_servers(), expected);
    let (files, runtime) = machine(Some(&home_models(SECRET)), Scripted::default(), &[]);
    prepare(&runtime, &config(), "/c/pi-agent", Some(&url)).unwrap().unwrap();
    assert_eq!(parse(&files.read_text("/c/pi-agent/mcp.json").unwrap()).unwrap(), expected);
    let grants = grants(&runtime, &config(), &route_for(443), Some(("/p", "/a")), Some(&url));
    assert_eq!(grants, ["--read", "/p", "--run-from", "pi", "--write", "/a", "--net-connect", "443", "--env", "PI_CODING_AGENT_DIR"].map(String::from));
}

#[test]
fn a_plan_command_the_process_could_not_start_is_the_critics_unexpected_error() {
    let process = Scripted::default().on("sbx-plan grants", Err("spawn sbx-plan ENOENT".to_string()));
    let (files, runtime) = machine(Some(&home_models(SECRET)), process, &[]);
    files.put("/c/prompt.md", "prompt");
    let roster = roster_critic();
    let launch = CriticLaunch { critic: &roster, packet_dir: "/c/packet".to_string(), prompt_path: "/c/prompt.md".to_string(), critic_dir: "/c".to_string(), schema_path: String::new(), toolset: READ_ONLY_TOOLSET };
    let error = pi_review(&runtime, &launch, &config()).expect_err("not started");
    assert!(format!("{error:?}").contains("spawn sbx-plan ENOENT"), "{error:?}");
}

/// A live check, run by hand on a machine with sbx, Pi and a LiteLLM key for the free route:
/// `cargo test -p critique -- --ignored a_searching_pi_critic_answers_in_a_real_sandbox_through_the_host_search_server`.
/// A free model is asked to call the packet search tool once; its call and the host server's answer must show in the stream.
#[test]
#[ignore = "needs sbx, pi and the free LiteLLM route on this machine"]
fn a_searching_pi_critic_answers_in_a_real_sandbox_through_the_host_search_server() {
    let runtime = crate::runtime::default_runtime();
    let dir = std::env::temp_dir().join(format!("critique-live-{}", std::process::id()));
    let packet = dir.join("packet");
    std::fs::create_dir_all(packet.join("tree")).unwrap();
    std::fs::write(packet.join("tree/notes.txt"), "alpha\nSENTINEL_976 is the line the host search server finds\nomega\n").unwrap();
    let prompt = dir.join("prompt.md");
    std::fs::write(&prompt, "Call the tool mcp__packet__grep exactly once with the pattern SENTINEL_976 and no other argument. Then reply with the single word DONE and nothing else.").unwrap();
    let config = crate::routing::PiCriticClient { binary: "pi".to_string(), provider: "litellm-free".to_string(), model: "free/gemma-4-31b-it".to_string(), family: "google".to_string(), data_scope: "public".to_string(), free: true, tier: None };
    let roster = RosterCritic { client: "live".to_string(), role: "r".to_string(), lenses: vec![], max_price_usd: 0.0, time_limit_s: 300.0, critic_id: "live".to_string(), client_config: CriticClient::Pi(config.clone()), fallbacks: vec![] };
    let toolset = CriticToolset::PiSearch {
        search: vec![SearchToolName::Grep, SearchToolName::List],
        limits: default_search_limits(),
        server: SearchServerLaunch { command: "unused".to_string(), args: vec![] },
    };
    let launch = CriticLaunch { critic: &roster, packet_dir: packet.to_string_lossy().into_owned(), prompt_path: prompt.to_string_lossy().into_owned(), critic_dir: dir.to_string_lossy().into_owned(), schema_path: String::new(), toolset };
    let report = pi_review(&runtime, &launch, &config).expect("a report");
    // The tool's own events: its start, and its end with what the host server answered.
    let events: Vec<&str> = report.stdout.lines().filter(|line| line.contains("tool_execution_") || line.contains("\"toolResult\"")).collect();
    let called = events.iter().any(|line| line.contains("\"toolName\":\"mcp__packet__grep\""));
    let answered = events.iter().any(|line| line.contains("SENTINEL_976 is the line the host search server finds"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(called && answered && report.exit_code == 0, "exit {} called {called} answered {answered}: {}", report.exit_code, events.join("\n").chars().take(3000).collect::<String>());
}
