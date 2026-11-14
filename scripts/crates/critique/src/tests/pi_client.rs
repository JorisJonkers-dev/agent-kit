//! The Pi critic client's readers, at the edges the goldens do not reach. The expected words are the node runner's,
//! from `adapters/pi`.

use crate::clients::CriticOutcome;
use crate::pi_client::{auth_readiness, classify_pi_run, outside_calls, outside_packet, read_pi_stream, PiToolCall};
use model_gateway::json::parse;
use crate::routing::PiCriticClient;
use model_gateway::ports::ProcessResult;

fn config() -> PiCriticClient {
    PiCriticClient { binary: "pi".to_string(), provider: "litellm-vibe-study".to_string(), model: "glm-5-3".to_string(), family: "zhipu".to_string() }
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
    let classify = |stdout: &str, timed_out: bool| failure(classify_pi_run(&result(stdout, timed_out), &read_pi_stream(stdout), "/p", 600.0));
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

// The confinement: the home's route, the key, the launcher and its check, as the node runner's `adapters/pi` reads them.

use super::vibe_client::{runtime_with, FakeFiles};
use crate::clients::{CriticLaunch, CriticReport};
use crate::critic_tools::READ_ONLY_TOOLSET;
use crate::pi_client::{
    confine_readiness, confined_command, grants, home_agent_dir, key, launcher, pi_command, pi_readiness, pi_review, prepare, route, route_of, url_port, PiRoute,
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

/// A machine with pi and agent-launcher on PATH, ready, and the home's providers as given.
fn machine(models: Option<&str>, process: Scripted, env: &[(&str, &str)]) -> (FakeFiles, crate::runtime::Runtime) {
    let files = FakeFiles::default();
    files.put("/bin/pi", "");
    files.put("/bin/agent-launcher", "");
    if let Some(models) = models {
        files.put(MODELS, models);
    }
    let mut all = vec![("HOME", HOME), ("PATH", "/bin")];
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
fn the_launcher_check_is_read_and_the_confined_command_line_built_with_or_without_a_path() {
    assert_eq!(confine_readiness(&Ok(ok("confine: ok"))).ready, true);
    let slow = ProcessResult { timed_out: true, ..ok("") };
    assert!(confine_readiness(&Ok(slow)).detail.unwrap().contains("no answer within 20s"));
    assert!(confine_readiness(&Err("spawn agent-launcher EACCES".to_string())).detail.unwrap().ends_with("so it does not run: spawn agent-launcher EACCES"));
    let roster = RosterCritic { client: "c".to_string(), role: "r".to_string(), lenses: vec![], max_price_usd: 1.0, time_limit_s: 1.0, critic_id: "c1".to_string(), client_config: CriticClient::Pi(config()), fallbacks: vec![] };
    let launch = CriticLaunch { critic: &roster, packet_dir: "/p".to_string(), prompt_path: "/c/prompt.md".to_string(), critic_dir: "/c".to_string(), schema_path: String::new(), toolset: READ_ONLY_TOOLSET };
    let pi = || pi_command(&config(), &launch, "prompt".to_string());
    let env = |command: ProcessCommand| command.env.get("PATH").cloned().unwrap_or_default();
    assert_eq!(env(confined_command("L", vec![], pi(), None, "/a", None)), "");
    assert_eq!(env(confined_command("L", vec![], pi(), Some(""), "/a", Some("/b"))), "/b");
    let full = confined_command("L", vec!["--read".to_string(), "/p".to_string()], pi(), Some("/usr/bin"), "/a", Some("/b"));
    assert_eq!((full.command.as_str(), &full.args[..4], full.cwd.as_deref(), full.stdin.as_deref()), ("L", &["confine", "--read", "/p", "--"].map(String::from)[..], Some("/p"), Some("prompt")));
    assert_eq!((env(full.clone()), full.env.get("PI_CODING_AGENT_DIR").cloned(), full.env.get("TMPDIR").cloned()), ("/b:/usr/bin".to_string(), Some("/a".to_string()), Some("/a".to_string())));
}

#[test]
fn the_home_pi_directory_and_the_launcher_follow_their_variables() {
    let (_, plain) = machine(None, Scripted::default(), &[]);
    assert_eq!((home_agent_dir(&plain), launcher(&plain)), ("/h/.pi/agent".to_string(), Some("agent-launcher".to_string())));
    let (_, named) = machine(None, Scripted::default(), &[("PI_CODING_AGENT_DIR", "/own"), ("AGENT_KIT_LAUNCHER_BIN", "/opt/agent-launcher")]);
    assert_eq!((home_agent_dir(&named), launcher(&named)), ("/own".to_string(), Some("/opt/agent-launcher".to_string())));
    let (files, gone) = machine(None, Scripted::default(), &[]);
    let _ = files.remove_file("/bin/agent-launcher");
    assert_eq!(launcher(&gone), None);
}

#[test]
fn the_grants_add_the_homes_pi_bin_and_a_launchs_packet_and_agent_directory() {
    let (files, runtime) = machine(None, Scripted::default(), &[]);
    assert_eq!(grants(&runtime, &config(), &route_for(443), None), ["--run-from", "pi", "--net-connect", "443"].map(String::from));
    files.put("/h/.pi/agent/bin", "");
    assert_eq!(
        grants(&runtime, &config(), &route_for(8443), Some(("/p", "/a"))),
        ["--read", "/p", "--run-from", "pi", "--run", "/h/.pi/agent/bin", "--write", "/a", "--net-connect", "8443"].map(String::from)
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
fn readiness_needs_the_route_the_launcher_and_a_check_that_passes() {
    let ready = |models: Option<&str>, process: Scripted, remove_launcher: bool| {
        let (files, runtime) = machine(models, process, &[]);
        if remove_launcher {
            let _ = files.remove_file("/bin/agent-launcher");
        }
        pi_readiness(&runtime, &config()).unwrap()
    };
    let models = home_models(SECRET);
    let checked = Scripted::default();
    assert!(ready(Some(&models), checked.clone(), false).ready);
    assert!(checked.seen().contains(&"agent-launcher confine --check --run-from pi --net-connect 443".to_string()), "{:?}", checked.seen());
    assert!(ready(None, Scripted::default(), false).detail.unwrap().contains("has no litellm-vibe-study provider"));
    assert!(ready(Some("not json"), Scripted::default(), false).detail.unwrap().contains("has no litellm-vibe-study provider"));
    assert!(ready(Some(&models), Scripted::default(), true).detail.unwrap().starts_with("pi: agent-launcher is not on PATH"));
    let refusing = Scripted::default().on("agent-launcher confine --check", Ok(failing(125, "agent-launcher: confine: no Landlock\n", "")));
    assert!(ready(Some(&models), refusing, false).detail.unwrap().ends_with("so it does not run: agent-launcher: confine: no Landlock"));
    let unoffered = Scripted::default().on("pi --offline", Ok(ok("litellm-vibe-study other 128K")));
    assert!(ready(Some(&models), unoffered, false).detail.unwrap().contains("does not offer glm-5-3"));
}

#[test]
fn prepare_writes_only_the_routes_provider_with_its_key_and_says_why_it_cannot() {
    let (files, runtime) = machine(Some(&home_models(SECRET)), Scripted::default(), &[]);
    let (launcher, prepared) = prepare(&runtime, &config(), "/c/pi-agent").unwrap().unwrap();
    assert_eq!((launcher.as_str(), prepared.port), ("agent-launcher", 443));
    let written = parse(&files.read_text("/c/pi-agent/models.json").unwrap()).unwrap();
    let expected = parse(r#"{"providers":{"litellm-vibe-study":{"api":"openai-completions","apiKey":"sk-route","baseUrl":"https://litellm.example.test/v1","models":[{"cost":{"input":0.5},"id":"glm-5-3"}]}}}"#).unwrap();
    assert_eq!((written, files.read_text("/c/pi-agent/settings.json").unwrap()), (expected, "{}\n".to_string()));
    let (_, no_route) = machine(None, Scripted::default(), &[]);
    assert!(prepare(&no_route, &config(), "/c/pi-agent").unwrap().unwrap_err().contains("has no litellm-vibe-study provider"));
    let (files, no_launcher) = machine(Some(&home_models(SECRET)), Scripted::default(), &[]);
    let _ = files.remove_file("/bin/agent-launcher");
    assert!(prepare(&no_launcher, &config(), "/c/pi-agent").unwrap().unwrap_err().starts_with("pi: agent-launcher is not on PATH"));
    let (_, bad_key) = machine(Some(&home_models("!sh -c x")), Scripted::default(), &[]);
    assert!(prepare(&bad_key, &config(), "/c/pi-agent").unwrap().unwrap_err().contains("a shell command"));
    assert!(route(&bad_key, &config()).is_ok());
}

#[test]
fn a_critic_that_cannot_be_confined_never_starts_and_its_report_says_why() {
    let process = Scripted::default();
    let (files, runtime) = machine(None, process.clone(), &[]);
    files.put("/c/prompt.md", "prompt");
    let roster = RosterCritic { client: "c".to_string(), role: "r".to_string(), lenses: vec![], max_price_usd: 1.0, time_limit_s: 1.0, critic_id: "c1".to_string(), client_config: CriticClient::Pi(config()), fallbacks: vec![] };
    let launch = CriticLaunch { critic: &roster, packet_dir: "/c/packet".to_string(), prompt_path: "/c/prompt.md".to_string(), critic_dir: "/c".to_string(), schema_path: String::new(), toolset: READ_ONLY_TOOLSET };
    let report: CriticReport = pi_review(&runtime, &launch, &config()).unwrap();
    assert_eq!((report.exit_code, report.charged_usd, report.export_path.as_str()), (125, 0.0, "/c/result.json"));
    assert_eq!(failure(report.outcome).0, "unconfined");
    assert!(!process.seen().iter().any(|line| line.contains("confine")));
}

#[test]
fn a_launch_the_launcher_refused_reads_as_unconfined() {
    let refused = |stderr: &str| failure(classify_pi_run(&failing(125, stderr, ""), &read_pi_stream(""), "/p", 1.0));
    assert_eq!(refused("agent-launcher: confine: no Landlock; nothing started\n"), ("unconfined".to_string(), "the operating system could not confine pi to its packet, so it did not start: agent-launcher: confine: no Landlock; nothing started".to_string()));
    assert!(refused("").1.ends_with("did not start: exit 125"));
}
