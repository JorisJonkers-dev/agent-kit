//! What the goldens do not reach of the Pi client: the pieces of a confined launch that only a particular machine
//! shows (a home with no `HOME`, a `bin` beside Pi, writing runs with git directories), the stream shapes Pi can
//! print, and the catalogue's groups.

use crate::client_run::{ClientRun, ClientRunError, ClientRunRequest};
use crate::error::GatewayError;
use crate::extras::RunExtras;
use crate::json::{self, Value};
use crate::launchers::{launcher, pi_grants, LaunchExtras, LaunchPaths, LaunchRequest, PiConfinement};
use crate::litellm_catalogue::parse_catalogue;
use crate::locations::pi_models_path;
use crate::models::ClientListing;
use crate::outputs::pi_output;
use crate::pi_launch::{home_agent_dir, prepare_pi, url_port, NO_LAUNCHER};
use crate::ports::{FileStamp, Files, FsError, Process, ProcessCommand, ProcessResult};
use crate::run::{is_pi_client, pi_provider, RunClient, RunMode};
use crate::sources::parse_pi_listing;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// Files in memory: what a test puts there, what a launch writes, and what it removes.
#[derive(Default)]
struct Disk {
    files: Mutex<BTreeMap<String, String>>,
    removed: Mutex<Vec<String>>,
    write_fails: bool,
}

impl Disk {
    fn with(self, path: &str, text: &str) -> Disk {
        self.files.lock().expect("files").insert(path.to_string(), text.to_string());
        self
    }
}

impl Files for Disk {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        self.files.lock().expect("files").get(path).cloned().ok_or(FsError { code: "ENOENT".into(), message: format!("ENOENT: {path}") })
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
        Ok(self.files.lock().expect("files").contains_key(path))
    }
    fn append(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn stamp(&self, _path: &str) -> Option<FileStamp> {
        None
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        if self.write_fails {
            return Err(FsError { code: "EACCES".into(), message: "EACCES: denied".into() });
        }
        self.files.lock().expect("files").insert(path.to_string(), content.to_string());
        Ok(())
    }
    fn write_text(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn temp_dir(&self, _parent: &str, _prefix: &str) -> Result<String, FsError> {
        Ok("/tmp/x".into())
    }
    fn remove(&self, path: &str) -> Result<(), FsError> {
        self.removed.lock().expect("removed").push(path.to_string());
        Ok(())
    }
}

/// A process that answers the secret store and the launcher with what a test says.
struct Programs {
    secret: Result<ProcessResult, String>,
    launched: Result<ProcessResult, String>,
    ran: Mutex<Vec<ProcessCommand>>,
}

fn result(stdout: &str, stderr: &str, exit_code: i32, timed_out: bool) -> Result<ProcessResult, String> {
    Ok(ProcessResult { stdout: stdout.into(), stderr: stderr.into(), exit_code, timed_out })
}

impl Programs {
    fn new(secret: Result<ProcessResult, String>) -> Programs {
        Programs { secret, launched: result("", "", 0, false), ran: Mutex::new(Vec::new()) }
    }
}

impl Process for Programs {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.secret.clone()
    }
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.ran.lock().expect("ran").push(command.clone());
        self.launched.clone()
    }
}

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
}

const MODELS: &str = "/home/me/.pi/agent/models.json";

fn provider(api_key: &str) -> String {
    format!(r#"{{"providers": {{"litellm-vibe": {{"api": "openai-completions", "apiKey": "{api_key}", "baseUrl": "http://127.0.0.1:4000/v1", "models": [{{"id": "other"}}, {{"id": "glm-5.3", "cost": {{"input": 1}}}}]}}}}}}"#)
}

fn disk(api_key: &str) -> Disk {
    Disk::default().with(MODELS, &provider(api_key)).with("/bin/agent-launcher", "")
}

fn home() -> HashMap<String, String> {
    env(&[("HOME", "/home/me"), ("PATH", "/bin")])
}

#[test]
fn a_pi_client_is_pi_and_the_route_names_its_provider() {
    for id in ["pi-vibe", "pi-opencode-go", "pi-9"] {
        assert!(is_pi_client(id), "{id}");
        assert_eq!(RunClient::parse(id), Some(RunClient::Pi));
    }
    for id in ["pi", "pi-", "pi-Vibe", "pi-vibe/x", "nobody"] {
        assert!(!is_pi_client(id), "{id}");
    }
    assert_eq!(RunClient::Pi.as_str(), "pi");
    assert_eq!(pi_provider("pi-opencode-go"), "litellm-opencode-go");
}

#[test]
fn the_proxy_port_is_the_one_after_the_host_else_the_schemes_own() {
    assert_eq!(url_port("https://litellm.example/v1"), Some(443));
    assert_eq!(url_port("http://litellm.example"), Some(80));
    assert_eq!(url_port("http://127.0.0.1:4000/v1"), Some(4000));
    for bad in ["http://host:0/v1", "http://host:70000/v1", "http://host:x/v1", "http://host:/v1", "ftp://host/v1"] {
        assert_eq!(url_port(bad), None, "{bad}");
    }
}

#[test]
fn the_home_directory_is_the_one_the_environment_names_else_dot_pi_agent() {
    assert_eq!(home_agent_dir(&env(&[("PI_CODING_AGENT_DIR", "/own"), ("HOME", "/h")])), "/own");
    assert_eq!(home_agent_dir(&env(&[("PI_CODING_AGENT_DIR", ""), ("HOME", "/h")])), "/h/.pi/agent");
    assert_eq!(home_agent_dir(&env(&[])), "/.pi/agent");
    assert_eq!(pi_models_path(&env(&[("PI_CODING_AGENT_DIR", "/own")])), Ok("/own/models.json".to_string()));
    assert_eq!(pi_models_path(&env(&[("HOME", "/h")])), Ok("/h/.pi/agent/models.json".to_string()));
    assert!(pi_models_path(&env(&[])).is_err());
}

#[test]
fn a_launch_is_prepared_with_the_key_read_just_now_and_the_homes_bin_when_there_is_one() {
    let files = disk("!agent-kit-secret get LITELLM_VIBE_KEY").with("/home/me/.pi/agent/bin", "");
    let programs = Programs::new(result("the-key\n", "", 0, false));

    let prepared = prepare_pi(&files, &programs, &home(), "litellm-vibe", "glm-5.3", "/run/pi-agent").expect("prepared");

    assert_eq!(
        prepared,
        PiConfinement {
            launcher: "agent-launcher".into(),
            agent_dir: "/run/pi-agent".into(),
            bin: Some("/home/me/.pi/agent/bin".into()),
            port: 4000,
            path: Some("/bin".into()),
        }
    );
    let written = json::parse(&files.read_text("/run/pi-agent/models.json").expect("models")).expect("json");
    let entry = written.as_object().and_then(|o| o.get("providers")).and_then(Value::as_object).and_then(|p| p.get("litellm-vibe")).and_then(Value::as_object).expect("provider").clone();
    assert_eq!(entry.get("apiKey").and_then(Value::as_str), Some("the-key"));
    assert_eq!(files.read_text("/run/pi-agent/settings.json").as_deref(), Ok("{}\n"));
}

#[test]
fn a_literal_key_and_a_model_the_home_does_not_list_and_the_launcher_the_environment_names_are_taken_as_they_are() {
    let files = Disk::default().with(MODELS, &provider("literal").replace("\"models\": [{\"id\": \"other\"}, {\"id\": \"glm-5.3\", \"cost\": {\"input\": 1}}]", "\"models\": 3"));
    let programs = Programs::new(Err("never asked".into()));

    let prepared = prepare_pi(&files, &programs, &env(&[("HOME", "/home/me"), ("AGENT_KIT_LAUNCHER_BIN", "/opt/launcher")]), "litellm-vibe", "unlisted", "/run/pi-agent").expect("prepared");

    assert_eq!(prepared, PiConfinement { launcher: "/opt/launcher".into(), agent_dir: "/run/pi-agent".into(), bin: None, port: 4000, path: None });
    assert!(files.read_text("/run/pi-agent/models.json").expect("models").contains("\"id\": \"unlisted\""));
}

#[test]
fn a_launch_that_cannot_be_prepared_says_why() {
    let programs = Programs::new(result("k", "", 0, false));
    let run = |files: &Disk, env: &HashMap<String, String>| prepare_pi(files, &programs, env, "litellm-vibe", "glm-5.3", "/run/pi-agent").expect_err("refused");
    assert!(run(&Disk::default(), &home()).contains("cannot be read"));
    assert!(run(&Disk::default().with(MODELS, "[]"), &home()).contains("has no litellm-vibe provider"));
    assert!(run(&Disk::default().with(MODELS, r#"{"providers": {"litellm-vibe": {"api": "x"}}}"#), &home()).contains("lacks a baseUrl, api or apiKey"));
    assert!(run(&Disk::default().with(MODELS, &provider("k").replace("http://127.0.0.1:4000", "ftp://x")), &home()).contains("not an http or https URL"));
    assert_eq!(run(&disk("k").with("/bin/agent-launcher", ""), &env(&[("HOME", "/home/me"), ("PATH", ":/nowhere")])), NO_LAUNCHER);
    assert_eq!(run(&disk("k"), &env(&[("HOME", "/home/me"), ("AGENT_KIT_LAUNCHER_BIN", "")])), NO_LAUNCHER);
    assert!(run(&disk("!echo a | cat"), &home()).contains("is a shell command the gateway does not run"));
    let denied = Disk { write_fails: true, ..disk("k") };
    assert!(run(&denied, &home()).contains("cannot write the launch's agent directory"));
}

#[test]
fn a_key_the_secret_store_cannot_give_is_named_without_any_of_the_key() {
    let files = disk("!agent-kit-secret get NAME");
    let refused = |secret: Result<ProcessResult, String>| prepare_pi(&files, &Programs::new(secret), &home(), "litellm-vibe", "glm-5.3", "/run/pi-agent").expect_err("refused");
    let says = "pi: the litellm-vibe key could not be read from the secret store (NAME): ";
    assert_eq!(refused(result("leak", "\n  no such secret\nmore", 1, false)), format!("{says}no such secret"));
    assert_eq!(refused(result("", "", 3, false)), format!("{says}exit 3"));
    assert_eq!(refused(result("  \n", "", 0, false)), format!("{says}exit 0"));
    assert_eq!(refused(result("late", "slow", 0, true)), format!("{says}slow"));
    assert_eq!(refused(Err("spawn failed".into())), format!("{says}spawn failed"));
}

fn request(client: RunClient, provider: Option<&str>, mode: RunMode) -> ClientRunRequest {
    ClientRunRequest {
        launch: LaunchRequest { model: "glm-5.3".into(), time_limit_s: 30.0, max_price_usd: None, mode: Some(mode), provider: provider.map(str::to_string) },
        binary: "pi".into(),
        client,
        prompt: "ask".into(),
        paths: LaunchPaths { export_dir: "/run/export".into(), prompt_path: "/run/prompt.md".into(), run_dir: None, work_dir: "/run/work".into(), git_writable: Some(vec!["/repo/.git".into()]) },
        extras: RunExtras::default(),
    }
}

#[test]
fn the_launch_removes_its_agent_directory_whether_or_not_pi_started() {
    let files = disk("literal");
    let mut programs = Programs::new(Err("never asked".into()));
    programs.launched = result("", "", 0, false);
    let outcome = (ClientRun { process: &programs, files: &files, env: &home() }).launch(&request(RunClient::Pi, Some("litellm-vibe"), RunMode::Write)).expect("launched");
    assert!(outcome.output.failure.is_some(), "an empty stream is a failed run");
    let command = programs.ran.lock().expect("ran")[0].clone();
    assert_eq!(command.command, "agent-launcher");
    assert_eq!(&command.args[..6], ["confine", "--write", "/run/work", "--write", "/repo/.git", "--run-from"]);
    assert_eq!(files.removed.lock().expect("removed").as_slice(), ["/run/export/pi-agent"]);

    let mut broken = Programs::new(Err("never asked".into()));
    broken.launched = Err("spawn failed".into());
    let files = disk("literal");
    let error = (ClientRun { process: &broken, files: &files, env: &home() }).launch(&request(RunClient::Pi, Some("litellm-vibe"), RunMode::ReadOnly)).expect_err("spawn");
    assert_eq!(error, ClientRunError::Spawn("spawn failed".into()));
    assert_eq!(files.removed.lock().expect("removed").as_slice(), ["/run/export/pi-agent"]);
}

#[test]
fn a_launch_that_names_no_provider_or_cannot_be_confined_never_starts() {
    let files = disk("literal");
    let programs = Programs::new(Err("never asked".into()));
    let run = |provider: Option<&str>| (ClientRun { process: &programs, files: &files, env: &home() }).launch(&request(RunClient::Pi, provider, RunMode::ReadOnly)).expect_err("refused");
    assert_eq!(run(None), ClientRunError::Gateway(GatewayError::new("unconfined", "a pi launch must name its provider")));
    assert!(matches!(run(Some("litellm-local")), ClientRunError::Gateway(GatewayError { code, .. }) if code == "unconfined"));
    assert!(programs.ran.lock().expect("ran").is_empty());
}

#[test]
fn a_bare_pi_command_is_what_the_builder_gives_when_nothing_confines_it_and_the_grants_name_every_directory() {
    let paths = LaunchPaths { export_dir: "/e".into(), prompt_path: "/p".into(), run_dir: None, work_dir: "/w".into(), git_writable: None };
    let input = LaunchRequest { model: "m".into(), time_limit_s: 30.0, max_price_usd: None, mode: None, provider: None };
    let bare = (launcher(RunClient::Pi).command)("pi", &input, &paths, "ask", None);
    assert_eq!((bare.command.as_str(), bare.cwd.as_deref(), bare.stdin.as_deref(), bare.timeout_ms), ("pi", Some("/w"), Some("ask"), Some(30_000)));
    assert!(bare.args.contains(&"undefined".to_string()));
    let extras = LaunchExtras { env: BTreeMap::from([("KEEP".to_string(), "1".to_string())]), ..LaunchExtras::default() };
    assert_eq!((launcher(RunClient::Pi).command)("pi", &input, &paths, "ask", Some(&extras)).env, extras.env);

    let confine = PiConfinement { launcher: "l".into(), agent_dir: "/a".into(), bin: Some("/b".into()), port: 8, path: Some(String::new()) };
    assert_eq!(pi_grants(&input, &paths, "pi", &confine), ["--read", "/w", "--run-from", "pi", "--run", "/b", "--write", "/a", "--net-connect", "8"]);
    let write = LaunchRequest { mode: Some(RunMode::Write), ..input };
    let git = LaunchPaths { git_writable: Some(vec!["/g".into()]), ..paths };
    assert_eq!(pi_grants(&write, &git, "pi", &PiConfinement { bin: None, ..confine.clone() }), ["--write", "/w", "--write", "/g", "--run-from", "pi", "--write", "/a", "--net-connect", "8"]);
    let confined = (launcher(RunClient::Pi).command)("pi", &write, &git, "ask", Some(&LaunchExtras { confine: Some(confine), ..LaunchExtras::default() }));
    assert_eq!(confined.env.get("PATH").map(String::as_str), Some("/b"));
}

#[test]
fn the_listing_of_a_models_file_is_its_providers_models_or_why_it_has_none() {
    let models = |text: &str| parse_pi_listing(text, "litellm-vibe", "/m.json");
    let ids = |listing: ClientListing| match listing {
        ClientListing::Models(models) => models.into_iter().map(|model| model.model).collect::<Vec<_>>(),
        ClientListing::Reason(reason) => vec![reason],
    };
    assert_eq!(ids(models(&provider("k"))), ["other", "glm-5.3"]);
    assert_eq!(ids(models(r#"{"providers": {"litellm-vibe": {"models": [{"id": 7}, {"id": ""}, 3, {"id": "x"}]}}}"#)), ["x"]);
    assert_eq!(ids(models("not json")), ["listing failed: /m.json is not JSON"]);
    for text in ["[]", r#"{"providers": {"litellm-vibe": {}}}"#, r#"{"providers": {}}"#] {
        assert!(ids(models(text))[0].contains("has no litellm-vibe provider with models"), "{text}");
    }
}

#[test]
fn a_pi_stream_is_read_whatever_shape_it_has() {
    let assistant = |message: &str| format!(r#"{{"type": "message_end", "message": {{"role": "assistant", {message}}}}}"#);
    let one = pi_output(&assistant(r#""content": [{"type": "text", "text": "done"}, {"type": "thinking"}], "usage": {"input": 3, "output": 2, "cacheRead": 1, "cost": {"total": 0.5}}"#));
    assert_eq!((one.answer.as_deref(), one.charged, one.tokens.input, one.tokens.cached, one.tokens.output), (Some("done"), Some(0.5), 4.0, 1.0, 2.0));
    assert_eq!(pi_output(&assistant(r#""content": "not a list""#)).answer, None);
    assert_eq!(pi_output(&assistant(r#""content": [{"type": "text", "text": "  "}]"#)).answer, None);
    let failed = pi_output(&assistant(r#""content": [{"type": "text", "text": "partial"}], "stopReason": "error", "errorMessage": "upstream said 500""#));
    assert_eq!((failed.failure.as_deref(), failed.error.map(|error| error.text), failed.answer.as_deref()), (Some("client-error"), Some("upstream said 500".to_string()), Some("partial")));
    assert_eq!(pi_output(&assistant(r#""stopReason": "aborted""#)).error.map(|error| error.text), Some("aborted".to_string()));
    for stream in ["", "not json", r#"{"type": "message_end"}"#, r#"{"type": "message_end", "message": []}"#, r#"{"type": "message_end", "message": {"role": "user"}}"#] {
        assert_eq!(pi_output(stream).failure.as_deref(), Some("client-error"), "{stream}");
    }
}

#[test]
fn a_deployment_that_names_no_order_is_tried_first() {
    let raw = json::parse(
        r#"{"updated_at": "x", "providers": {}, "groups": {"g": {"deployments": [{"route": "vibe", "upstream_id": "m"}, {"route": 5}, 3]}, "bad": 3, "none": {}}}"#,
    )
    .expect("json");
    let catalogue = parse_catalogue(&raw).expect("catalogue");
    assert_eq!(catalogue.groups.keys().collect::<Vec<_>>(), ["g"]);
    assert_eq!(catalogue.groups["g"].deployments.iter().map(|d| d.order).collect::<Vec<_>>(), [0]);
    let none = parse_catalogue(&json::parse(r#"{"updated_at": "x", "providers": {}, "groups": 3}"#).expect("json")).expect("catalogue");
    assert!(none.groups.is_empty());
}

#[test]
fn a_machine_with_no_home_lists_no_pi_models_and_says_why() {
    use super::config::{runtime, StubFiles, StubGit};
    let listing = crate::sources::list_client(&runtime(StubFiles::new(), StubGit::new(None, &[]), HashMap::new()), "pi-vibe");
    assert!(matches!(listing, Some(ClientListing::Reason(reason)) if reason.starts_with("listing failed: ")));
    let missing = crate::sources::list_client(&runtime(StubFiles::new(), StubGit::new(None, &[]), env(&[("HOME", "/h")])), "pi-vibe");
    assert!(matches!(missing, Some(ClientListing::Reason(reason)) if reason.contains("/h/.pi/agent/models.json cannot be read")));
}
