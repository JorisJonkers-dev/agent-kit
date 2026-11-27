//! The sandbox backend a launch runs in, and the sbx plan a dry run turns a confined launch into.

use crate::json::{self, Value};
use crate::ports::{Clock, ProcessCommand};
use crate::sandbox::{dry_run, Mount, SandboxBackend, SbxPlan, SANDBOX_VARIABLE};
use std::collections::HashMap;

/// The clock every sbx launch test runs at: 2026-11-27 12:00 UTC, so a tag dated the 25th is two days old.
const SBX_NOW: i64 = 1_795_780_800_000;

struct Fixed;

impl Clock for Fixed {
    fn now_ms(&self) -> i64 {
        SBX_NOW
    }
    fn monotonic_ms(&self) -> f64 {
        0.0
    }
}

fn env(value: Option<&str>) -> HashMap<String, String> {
    value.map(|value| HashMap::from([(SANDBOX_VARIABLE.to_string(), value.to_string())])).unwrap_or_default()
}

fn confined(grants: &[&str]) -> ProcessCommand {
    let mut args = vec!["confine"];
    args.extend(grants);
    args.extend(["--", "opencode", "run", "say ok"]);
    ProcessCommand { cwd: Some("/work".to_string()), ..ProcessCommand::new("/bin/agent-launcher", &args) }
}

#[test]
fn confine_is_the_backend_unless_the_machine_names_another_and_an_unknown_name_is_refused() {
    assert_eq!(SandboxBackend::select(&env(None)), Ok(SandboxBackend::Confine));
    assert_eq!(SandboxBackend::select(&env(Some(""))), Ok(SandboxBackend::Confine));
    assert_eq!(SandboxBackend::select(&env(Some("confine"))), Ok(SandboxBackend::Confine));
    assert_eq!(SandboxBackend::select(&env(Some("sbx-dry-run"))), Ok(SandboxBackend::SbxDryRun));
    assert_eq!(SandboxBackend::select(&env(Some("sbx"))), Ok(SandboxBackend::Sbx));
    assert_eq!(SandboxBackend::Sbx.as_str(), "sbx");
    let refused = SandboxBackend::select(&env(Some("vm"))).expect_err("not a backend");
    assert_eq!(refused.code, "sandbox-unknown");
    assert!(refused.message.contains("AGENT_KIT_SANDBOX=vm names no sandbox backend; use confine, sbx-dry-run or sbx"), "{}", refused.message);
    assert_eq!(SandboxBackend::Confine.as_str(), "confine");
}

#[test]
fn a_read_only_launch_mounts_its_grants_read_only_and_its_run_directory_written() {
    let command = confined(&["--read", "/work", "--read", "/repo/.git/worktrees/w", "--run-from", "opencode", "--run", "/opt/git", "--write", "/run/export", "--net-connect", "443", "--net-loopback", "--env", "HOME"]);
    let plan = SbxPlan::of_command("opencode", &command).expect("a confined launch");
    assert_eq!(plan.template, "docker.io/sbx/opencode-image");
    assert_eq!(plan.workdir, "/work");
    assert_eq!(plan.mounts.iter().map(Mount::arg).collect::<Vec<_>>(), ["/work:ro", "/repo/.git/worktrees/w:ro", "/run/export"]);
    assert_eq!(plan.programs, ["opencode", "/opt/git"]);
    assert_eq!(plan.env, ["HOME"]);
    assert_eq!(plan.ports, [443]);
    assert!(plan.loopback);
    // The port rule has no host names yet: that is said, not dropped.
    assert_eq!(plan.gaps.len(), 1);
    assert!(plan.gaps[0].starts_with("network: confine allows TCP ports 443 to any host"), "{}", plan.gaps[0]);
}

#[test]
fn a_path_granted_both_ways_is_written_once_and_the_keychain_is_a_gap() {
    let command = confined(&["--read", "/work", "--write", "/work", "--read", "/work", "--keychain", "--preferences"]);
    let plan = SbxPlan::of_command("codex", &command).expect("a confined launch");
    assert_eq!(plan.mounts, [Mount { path: "/work".to_string(), read_only: false }]);
    assert_eq!(plan.ports, Vec::<u16>::new());
    assert_eq!(plan.gaps.len(), 1);
    assert!(plan.gaps[0].starts_with("keychain:"), "{}", plan.gaps[0]);
}

#[test]
fn a_grant_the_plan_does_not_know_and_a_port_that_is_not_one_are_gaps() {
    let command = confined(&["--net-connect", "https", "--mystery", "x", "--env"]);
    let plan = SbxPlan::of_command("pi", &command).expect("a confined launch");
    assert_eq!(plan.gaps, ["--net-connect https is not a port", "--mystery x: a grant the plan does not know"]);
    // A flag cut off at the end takes an empty value rather than the `--` after it.
    assert_eq!(plan.env, [""]);
}

#[test]
fn a_launch_not_behind_the_launcher_has_no_plan() {
    assert_eq!(SbxPlan::of_command("claude", &ProcessCommand::new("claude", &["-p"])), None);
    assert_eq!(SbxPlan::of_command("claude", &ProcessCommand::new("claude", &[])), None);
    // `confine` with no `--` is not a launch the launcher would take.
    assert_eq!(SbxPlan::of_command("claude", &ProcessCommand::new("/bin/agent-launcher", &["confine", "--read", "/work"])), None);
    let refusal = dry_run("claude", &ProcessCommand::new("claude", &["-p"]));
    assert_eq!(refusal.code, "sandbox-dry-run");
    assert!(refusal.message.ends_with("there are no grants to plan from"), "{}", refusal.message);
}

#[test]
fn a_dry_run_refuses_with_the_plan_as_json_and_never_a_value() {
    let refusal = dry_run("opencode", &confined(&["--write", "/work", "--net-connect", "4000", "--env", "LITELLM_KEY"]));
    assert_eq!(refusal.code, "sandbox-dry-run");
    let (said, plan) = refusal.message.split_once("the sbx plan: ").expect("the plan");
    assert_eq!(said, "opencode: not started (AGENT_KIT_SANDBOX=sbx-dry-run); ");
    let plan = json::parse(plan).expect("JSON");
    let object = plan.as_object().expect("an object");
    assert_eq!(object.get("backend").and_then(|value| value.as_str()), Some("sbx-dry-run"));
    assert_eq!(object.get("mounts"), Some(&json::parse(r#"["/work"]"#).expect("JSON")));
    assert_eq!(object.get("env"), Some(&json::parse(r#"["LITELLM_KEY"]"#).expect("JSON")));
    assert_eq!(object.get("ports"), Some(&json::parse("[4000]").expect("JSON")));
    assert_eq!(object.get("loopback"), Some(&json::parse("false").expect("JSON")));
}

#[test]
fn a_path_granted_read_only_twice_stays_read_only() {
    let command = confined(&["--read", "/work", "--read", "/work"]);
    let plan = SbxPlan::of_command("pi", &command).expect("a confined launch");
    assert_eq!(plan.mounts, [Mount { path: "/work".to_string(), read_only: true }]);
    assert_eq!(plan.mounts[0].arg(), "/work:ro");
}

#[test]
fn the_sbx_backend_is_named_sbx_dry_run_in_the_plan() {
    assert_eq!(SandboxBackend::SbxDryRun.as_str(), "sbx-dry-run");
}

#[test]
fn the_plan_json_carries_every_field_with_the_grants_it_was_built_from() {
    let command = confined(&["--read", "/work", "--write", "/run/export", "--run-from", "opencode", "--run", "/opt/git", "--env", "HOME", "--net-connect", "443", "--net-connect", "https", "--net-loopback", "--keychain"]);
    let plan = SbxPlan::of_command("opencode", &command).expect("a confined launch");
    let json = plan.to_json();
    let object = json.as_object().expect("an object");
    let text = |value: &str| Value::str(value);
    let list = |items: &[&str]| Value::Arr(items.iter().map(|item| Value::str(item)).collect());
    assert_eq!(object.get("backend"), Some(&text("sbx-dry-run")));
    assert_eq!(object.get("client"), Some(&text("opencode")));
    assert_eq!(object.get("template"), Some(&text("docker.io/sbx/opencode-image")));
    assert_eq!(object.get("workdir"), Some(&text("/work")));
    assert_eq!(object.get("mounts"), Some(&list(&["/work:ro", "/run/export"])));
    assert_eq!(object.get("programs"), Some(&list(&["opencode", "/opt/git"])));
    assert_eq!(object.get("env"), Some(&list(&["HOME"])));
    assert_eq!(object.get("ports"), Some(&Value::Arr(vec![Value::Num(443.0)])));
    assert_eq!(object.get("loopback"), Some(&Value::Bool(true)));
    // The https grant is a gap, the keychain is a gap, and the port rule is a gap: three, in grant order.
    assert_eq!(object.get("gaps"), Some(&Value::Arr(plan.gaps.iter().map(|gap| Value::str(gap)).collect())));
    assert_eq!(plan.gaps.len(), 3);
    assert_eq!(plan.gaps[0], "--net-connect https is not a port");
    assert!(plan.gaps[1].starts_with("keychain:"), "{}", plan.gaps[1]);
    assert!(plan.gaps[2].starts_with("network: confine allows TCP ports 443 to any host"), "{}", plan.gaps[2]);
}

// The sbx backend: every `sbx` call goes through the fake process, which records it and answers from the test.

use crate::client_run::{ClientRun, ClientRunError, ClientRunRequest};
use crate::extras::RunExtras;
use crate::launchers::{LaunchPaths, LaunchRequest};
use crate::error::GatewayError;
use crate::ports::{Ending, FileStamp, Files, FsError, LaunchWatch, Launched, Process, ProcessResult, Stream};
use crate::run::{RunClient, RunMode};
use crate::sbx_launch::{self, SbxLaunch};
use std::sync::Mutex;

type Answer = Box<dyn Fn(&[&str]) -> Result<ProcessResult, String> + Send + Sync>;

/// A process that records every program it is asked to run and answers `sbx` from two closures: `exec` for the
/// calls that wait for their status, `run` for the client's own launch.
struct Sbx {
    calls: Mutex<Vec<String>>,
    exec: Answer,
    run: Answer,
    /// How the client's launch ended when it did not exit by itself (a stop of the gateway, or the time limit), and whether it timed out.
    end: Option<(Ending, bool)>,
    /// The gateway pids that are still running.
    alive: Vec<u32>,
}

fn answer(stdout: &str, stderr: &str, exit_code: i32) -> Result<ProcessResult, String> {
    Ok(ProcessResult { stdout: stdout.to_string(), stderr: stderr.to_string(), exit_code, timed_out: false })
}

/// Every `sbx` call succeeds, and the client prints `ok`.
fn healthy(args: &[&str]) -> Result<ProcessResult, String> {
    // The one `curl` a launch makes is the Docker Hub listing of the client's image.
    if args.first() == Some(&"-fsS") {
        return answer(HUB_TAGS, "", 0);
    }
    answer("", "", 0)
}

/// The tags Docker Hub lists for a client's image: one too new (the 26th), one old enough (the 25th), and two that
/// are not dated tags at all.
pub(super) const HUB_TAGS: &str = r#"{"results":[{"name":"latest","digest":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","last_updated":"2026-11-27T01:00:00Z"},{"name":"20261126-c279c9da843e51b135a5be3750ffcdf8078485b4","digest":"sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210","last_updated":"2026-11-26T09:00:00Z"},{"name":"20261125-c279c9da843e51b135a5be3750ffcdf8078485b4","digest":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","last_updated":"2026-11-25T09:00:00Z"}]}"#;

/// The pinned image the chosen tag resolves to.
const PI_IMAGE: &str = "docker.io/sbx/pi-image@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const VIBE_IMAGE: &str = "docker.io/sbx/vibe-image@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OPENCODE_IMAGE: &str = "docker.io/sbx/opencode-image@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

impl Sbx {
    fn new(exec: impl Fn(&[&str]) -> Result<ProcessResult, String> + Send + Sync + 'static) -> Sbx {
        Sbx { calls: Mutex::new(Vec::new()), exec: Box::new(exec), run: Box::new(|_| answer("ok\n", "", 0)), end: None, alive: Vec::new() }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
}

impl Process for Sbx {
    fn run_watched(&self, command: &ProcessCommand, watch: &dyn LaunchWatch) -> Result<Launched, String> {
        let mut result = self.run(command)?;
        watch.output(Stream::Stdout, result.stdout.as_bytes());
        let ending = match &self.end {
            Some((ending, timed_out)) => {
                result.timed_out = *timed_out;
                result.exit_code = 143;
                ending.clone()
            }
            None => Ending::default(),
        };
        Ok(Launched { result, ending })
    }
    fn is_alive(&self, pid: u32) -> bool {
        self.alive.contains(&pid)
    }
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn exec(&self, command: &str, args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.calls.lock().expect("calls").push(format!("{command} {}", args.join(" ")));
        (self.exec)(args)
    }
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        self.calls.lock().expect("calls").push(format!("{} {}", command.command, args.join(" ")));
        (self.run)(&args)
    }
}

/// A file system in memory: what the gateway writes, what it removes.
#[derive(Default)]
struct Disk {
    files: Mutex<HashMap<String, String>>,
    removed: Mutex<Vec<String>>,
    /// A file that cannot be read once the launch has written it, as one the owner removed under it would be.
    unreadable: Option<String>,
    /// Refuses the one write a loopback route makes: a config whose content names the sandbox's host.
    refuse_rewrite: bool,
    /// Refuses the write of an sbx env file, and the removal of one.
    refuse_env_file: bool,
    refuse_env_remove: bool,
}

impl Files for Disk {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        if self.unreadable.as_deref() == Some(path) {
            return Err(FsError { code: "ENOENT".into(), message: format!("ENOENT: {path}") });
        }
        self.files.lock().expect("files").get(path).cloned().ok_or(FsError { code: "ENOENT".into(), message: format!("ENOENT: {path}") })
    }
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.write_private(path, content)
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
        let files = self.files.lock().expect("files");
        Ok(files.contains_key(path) || files.keys().any(|key| key.starts_with(&format!("{path}/"))))
    }
    fn append(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn stamp(&self, _path: &str) -> Option<FileStamp> {
        None
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        if self.refuse_rewrite && content.contains("host.docker.internal") || self.refuse_env_file && path.ends_with("sbx.env") {
            return Err(FsError { code: "EACCES".into(), message: format!("EACCES: {path}") });
        }
        self.files.lock().expect("files").insert(path.to_string(), content.to_string());
        Ok(())
    }
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.write_private(path, content)
    }
    fn copy_tree(&self, from: &str, to: &str) -> Result<(), FsError> {
        let mut files = self.files.lock().expect("files");
        let copied: Vec<(String, String)> = files.iter().filter(|(path, _)| path.as_str() == from).map(|(_, text)| (to.to_string(), text.clone())).collect();
        files.extend(copied);
        Ok(())
    }
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
        Ok(format!("{parent}/{prefix}-tmp"))
    }
    fn remove(&self, path: &str) -> Result<(), FsError> {
        self.removed.lock().expect("removed").push(path.to_string());
        if self.refuse_env_remove && path.ends_with("sbx.env") {
            return Err(FsError { code: "EBUSY".into(), message: format!("EBUSY: {path}") });
        }
        Ok(())
    }
}

/// What a watch saw: the notes the runner made.
#[derive(Default)]
struct Notes(Mutex<Vec<String>>);

impl LaunchWatch for Notes {
    fn spawned(&self, _pid: u32) {}
    fn output(&self, _stream: Stream, _chunk: &[u8]) {}
    fn note(&self, text: &str) {
        self.0.lock().expect("notes").push(text.to_string());
    }
}

const PI_MODELS: &str = r#"{"providers":{"litellm-vibe":{"api":"openai-completions","apiKey":"literal-key","baseUrl":"BASE","models":[{"id":"glm-5.3"}]}}}"#;

/// The owner's home with one LiteLLM provider on `base`, and the sbx backend selected.
fn pi_disk(base: &str) -> Disk {
    let disk = Disk::default();
    disk.files.lock().expect("files").insert("/home/me/.pi/agent/models.json".to_string(), PI_MODELS.replace("BASE", base));
    disk
}

fn sbx_env() -> HashMap<String, String> {
    HashMap::from([
        ("HOME".to_string(), "/home/me".to_string()),
        ("AGENT_KIT_LAUNCHER_BIN".to_string(), "/opt/agent-launcher".to_string()),
        (SANDBOX_VARIABLE.to_string(), "sbx".to_string()),
    ])
}

fn pi_request(client: RunClient, mode: Option<RunMode>) -> ClientRunRequest {
    let paths = LaunchPaths { export_dir: "/run/export".to_string(), prompt_path: "/run/prompt.md".to_string(), work_dir: "/run/work".to_string(), ..LaunchPaths::default() };
    let launch = LaunchRequest { model: "glm-5.3".to_string(), time_limit_s: 900.0, max_price_usd: None, mode, provider: Some("litellm-vibe".to_string()) };
    ClientRunRequest { launch, binary: "pi".to_string(), client, prompt: "say ok".to_string(), paths, extras: RunExtras::default() }
}

fn gateway_code(error: ClientRunError) -> (String, String) {
    match error {
        ClientRunError::Gateway(error) => (error.code, error.message),
        other => panic!("a refusal, not {other:?}"),
    }
}

fn sbx_name_in(calls: &[String]) -> String {
    let create = calls.iter().find(|call| call.starts_with("sbx create ")).expect("a create call");
    create.split_whitespace().skip_while(|word| *word != "--name").nth(1).expect("a name").to_string()
}

#[test]
fn sbx_is_a_selectable_backend_and_only_pi_and_opencode_read_only_runs_reach_it() {
    assert_eq!(SandboxBackend::select(&env(Some("sbx"))), Ok(SandboxBackend::Sbx));
    assert_eq!(sbx_launch::program_of(RunClient::Pi), Some("pi"));
    assert_eq!(sbx_launch::program_of(RunClient::Opencode), Some("opencode"));
    assert_eq!(sbx_launch::program_of(RunClient::Claude), None);
}

#[test]
fn every_refusal_before_a_launch_makes_nothing_and_names_its_reason() {
    let disk = pi_disk("http://localhost:4000/v1");
    let run = |process: &Sbx, client: RunClient, mode: Option<RunMode>| {
        let env = sbx_env();
        gateway_code(ClientRun { process, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(client, mode)).expect_err("refused"))
    };
    // A client the backend does not run is refused before sbx is even asked.
    let healthy_sbx = Sbx::new(healthy);
    let (code, message) = run(&healthy_sbx, RunClient::Claude, Some(RunMode::ReadOnly));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.contains("runs only pi, opencode and vibe"), "{message}");
    assert!(healthy_sbx.calls().is_empty(), "nothing asked of sbx");

    // sbx missing, and sbx that does not answer, are refused with the install command and no create.
    let missing = Sbx::new(|args| if args[0] == "version" { Err("sbx: executable file not found".to_string()) } else { healthy(args) });
    let (code, message) = run(&missing, RunClient::Pi, Some(RunMode::ReadOnly));
    assert_eq!(code, "sandbox-unavailable");
    assert!(message.contains("brew trust docker/tap && brew install docker/tap/sbx") && message.contains("sbx login"), "{message}");
    assert_eq!(missing.calls(), ["sbx version"]);
    let failing = Sbx::new(|args| if args[0] == "version" { answer("", "not signed in", 1) } else { healthy(args) });
    let (code, message) = run(&failing, RunClient::Pi, Some(RunMode::ReadOnly));
    assert_eq!(code, "sandbox-unavailable");
    assert!(message.contains("sbx version exited 1"), "{message}");
    assert_eq!(failing.calls(), ["sbx version"]);
}

#[test]
fn a_read_only_pi_run_creates_allows_execs_and_removes_its_sandbox_in_that_order() {
    let disk = pi_disk("http://localhost:4000/v1");
    let sbx = Sbx::new(healthy);
    let env = sbx_env();
    let notes = Notes::default();
    let outcome = ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch_watched(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly)), None, &notes).expect("a run");
    assert_eq!(outcome.result.stdout, "ok\n");
    assert!(outcome.warnings.is_empty());
    let calls = sbx.calls();
    let name = sbx_name_in(&calls);
    assert!(name.starts_with("mg-"), "{name}");
    assert_eq!(calls.len(), 7, "{calls:?}");
    assert_eq!(calls[0], "sbx version");
    assert_eq!(calls[1], "sbx ls -q");
    let calls: Vec<String> = calls.into_iter().skip(1).collect();
    assert!(calls[1].starts_with("curl -fsS https://hub.docker.com/v2/repositories/sbx/pi-image/tags"), "{}", calls[1]);
    assert_eq!(calls[2], format!("sbx create --name {name} --no-input --pull missing -t {PI_IMAGE} shell /run/export/pi-agent /run/work:ro"));
    assert_eq!(calls[3], format!("sbx policy allow network --sandbox {name} host.docker.internal:4000,localhost:4000"));
    // The client runs with its stdin open (the prompt), in the workdir; its two variables come from the env file, never the arguments.
    let exec = format!("sbx exec -i -w /run/work --env-file /run/export/pi-agent/sbx.env {name} pi -p --mode json ");
    assert!(calls[4].starts_with(&exec), "{}", calls[4]);
    assert!(calls[4].contains("--provider litellm-vibe --model glm-5.3 --tools read,grep,find,ls"), "{}", calls[4]);
    assert!(!calls[4].contains("HOME") && !calls[4].contains("PATH=") && !calls[4].contains("PI_CODING_AGENT_DIR"), "{}", calls[4]);
    assert_eq!(calls[5], format!("sbx rm --force {name}"));
    let env_file = disk.files.lock().expect("files").get("/run/export/pi-agent/sbx.env").cloned().expect("the env file was written");
    assert_eq!(env_file, "PI_CODING_AGENT_DIR=/run/export/pi-agent\nTMPDIR=/run/export/pi-agent\n");
    assert!(disk.removed.lock().expect("removed").contains(&"/run/export/pi-agent/sbx.env".to_string()), "the env file is removed after the run");
    // The loopback proxy is rewritten to the name the sandbox sees, in the launch's own file.
    assert!(disk.files.lock().expect("files").get("/run/export/pi-agent/models.json").expect("the agent directory's models").contains("http://host.docker.internal:4000/v1"));
    assert!(notes.0.lock().expect("notes").iter().any(|note| note.contains(&name) && note.contains("(sbx)")));
}

#[test]
fn a_failed_exec_still_removes_the_sandbox_and_is_the_runs_error() {
    let disk = pi_disk("http://localhost:4000/v1");
    let sbx = Sbx { run: Box::new(|_| Err("the program could not start".to_string())), ..Sbx::new(healthy) };
    let env = sbx_env();
    let error = ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly))).expect_err("the exec failed");
    let (code, message) = match error {
        ClientRunError::Spawn(message) => ("spawn".to_string(), message),
        other => panic!("{other:?}"),
    };
    assert_eq!(code, "spawn");
    assert!(message.starts_with("sbx run failed: the program could not start"), "{message}");
    let calls = sbx.calls();
    assert!(calls.last().expect("calls").starts_with("sbx rm --force mg-"), "{calls:?}");
}

#[test]
fn a_failed_create_names_its_reason_and_still_removes_what_it_named() {
    let disk = pi_disk("http://localhost:4000/v1");
    let sbx = Sbx::new(|args| if args[0] == "create" { answer("", "no room for a microVM\n", 1) } else { healthy(args) });
    let env = sbx_env();
    let error = ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly))).expect_err("the create failed");
    let ClientRunError::Spawn(message) = error else { panic!("a spawn error") };
    assert!(message.contains("sbx create exited 1: no room for a microVM"), "{message}");
    let calls = sbx.calls();
    assert_eq!(calls.len(), 5, "{calls:?}");
    assert!(calls[4].starts_with("sbx rm --force mg-"), "{calls:?}");
    assert!(!calls.iter().any(|call| call.starts_with("sbx exec")), "{calls:?}");
}

#[test]
fn a_failed_removal_is_a_warning_the_outcome_carries_not_a_panic() {
    let disk = pi_disk("http://localhost:4000/v1");
    let env = sbx_env();
    let exited = Sbx::new(|args| if args[0] == "rm" { answer("", "busy", 1) } else { healthy(args) });
    let outcome = ClientRun { process: &exited, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly))).expect("the run");
    assert_eq!(outcome.result.stdout, "ok\n");
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
    assert!(outcome.warnings[0].contains("was not removed (sbx rm exited 1)") && outcome.warnings[0].contains("sbx rm --force mg-"), "{:?}", outcome.warnings);

    let unrun = Sbx::new(|args| if args[0] == "rm" { Err("sbx rm did not run".to_string()) } else { healthy(args) });
    let outcome = ClientRun { process: &unrun, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly))).expect("the run");
    assert!(outcome.warnings[0].contains("(sbx rm did not run)"), "{:?}", outcome.warnings);
}

#[test]
fn sbx_own_exit_statuses_are_said_to_be_sbxs_and_the_clients_pass_through() {
    let disk = pi_disk("http://localhost:4000/v1");
    let env = sbx_env();
    let sbx = Sbx { run: Box::new(|_| answer("", "", 125)), ..Sbx::new(healthy) };
    let notes = Notes::default();
    let outcome = ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch_watched(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly)), None, &notes).expect("the run");
    assert_eq!(outcome.result.exit_code, 125);
    assert!(notes.0.lock().expect("notes").iter().any(|note| note.contains("sbx could not run pi (exit 125)")));
    let sbx = Sbx { run: Box::new(|_| answer("", "", 3)), ..Sbx::new(healthy) };
    let outcome = ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly))).expect("the run");
    assert_eq!(outcome.result.exit_code, 3, "the client's own status passes through");
}

/// An OpenCode launch's confined command with its keyed wrapper, as `launchers` builds it for a LiteLLM route.
fn keyed_opencode(with_env: bool) -> ProcessCommand {
    let mut words = vec!["-c", crate::launchers::OPENCODE_KEYED, "opencode-keyed", "agent-kit-secret", "LITELLM_KEY"];
    if with_env {
        words.extend(["env", "XDG_CONFIG_HOME=/run/export.client-home/.config"]);
    }
    words.extend(["/opt/agent-launcher", "confine", "--read", "/run/work", "--write", "/run/export.client-home", "--net-connect", "443", "--net-loopback", "--env", "LITELLM_KEY", "--env", "XDG_CONFIG_HOME", "--", "opencode", "run", "--standalone", "--model", "litellm-x/m", "--format", "json", "say ok"]);
    ProcessCommand { cwd: Some("/run/work".to_string()), ..ProcessCommand::new("/bin/sh", &words) }
}

const ROUTE_REMOTE: &str = r#"{"provider":{"litellm-x":{"options":{"baseURL":"https://litellm.example.com/v1"}}}}"#;

#[test]
fn an_opencode_keyed_launch_runs_its_client_in_the_opencode_template_with_the_key_read_and_the_remote_route_allowed() {
    let read = |key: &str| {
        assert_eq!(key, "LITELLM_KEY");
        Ok("secret-value".to_string())
    };
    let sbx = sbx_launch::translate(RunClient::Opencode, "mg-test", &keyed_opencode(true), ROUTE_REMOTE, &read).expect("a translation");
    assert_eq!(sbx.program, "opencode");
    assert_eq!(sbx.args, ["run", "--standalone", "--model", "litellm-x/m", "--format", "json", "say ok"]);
    // The plan's order: the variables in the order the launcher was given them.
    assert_eq!(sbx.env, ["LITELLM_KEY=secret-value", "XDG_CONFIG_HOME=/run/export.client-home/.config"]);
    assert_eq!(sbx.allow, ["litellm.example.com:443"]);
    let pinned = SbxLaunch { image: OPENCODE_IMAGE.to_string(), ..sbx.clone() };
    assert_eq!(pinned.create_args(), ["create", "--name", "mg-test", "--no-input", "--pull", "missing", "-t", OPENCODE_IMAGE, "shell", "/run/export.client-home", "/run/work:ro"]);
    assert_eq!(sbx.plan.workdir, "/run/work");

    // Without the wrapper's env pairs the launch's own environment is the one the values come from.
    let plain = ProcessCommand { env: [("XDG_CONFIG_HOME".to_string(), "/x".to_string())].into_iter().collect(), ..keyed_opencode(false) };
    let sbx = sbx_launch::translate(RunClient::Opencode, "mg-test", &plain, ROUTE_REMOTE, &read).expect("a translation");
    assert_eq!(sbx.env, ["LITELLM_KEY=secret-value", "XDG_CONFIG_HOME=/x"]);
}

#[test]
fn a_key_the_wrapper_would_read_that_cannot_be_read_refuses_the_launch() {
    let refused = sbx_launch::translate(RunClient::Opencode, "mg-test", &keyed_opencode(true), ROUTE_REMOTE, &|_| Err(GatewayError::new("sandbox-unsupported", "no key"))).expect_err("refused");
    assert_eq!(refused.code, "sandbox-unsupported");
    assert_eq!(refused.message, "no key");

    let process = Sbx::new(|_| answer("", "no such key", 1));
    let refused = sbx_launch::read_key(&process, "LITELLM_KEY").expect_err("no key");
    assert_eq!(refused.message, "the key LITELLM_KEY could not be read from the secret store");
    let empty = Sbx::new(|_| answer("  \n", "", 0));
    assert!(sbx_launch::read_key(&empty, "LITELLM_KEY").is_err());
    let unrun = Sbx::new(|_| Err("no secret tool".to_string()));
    assert!(sbx_launch::read_key(&unrun, "LITELLM_KEY").is_err());
    let good = Sbx::new(|_| answer("value\n", "", 0));
    assert_eq!(sbx_launch::read_key(&good, "LITELLM_KEY").expect("the key"), "value");
    assert_eq!(good.calls(), ["agent-kit-secret get LITELLM_KEY"]);
}

#[test]
fn a_command_the_sbx_plan_cannot_hold_is_refused_before_any_sandbox_is_named() {
    let translate = |client: RunClient, command: &ProcessCommand, route: &str| sbx_launch::translate(client, "mg-test", command, route, &|_| Ok(String::new()));
    let refusal = |result: Result<SbxLaunch, GatewayError>| {
        let error = result.expect_err("refused");
        (error.code, error.message)
    };
    let not_confined = ProcessCommand::new("opencode", &["run"]);
    let (code, message) = refusal(translate(RunClient::Opencode, &not_confined, ROUTE_REMOTE));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.contains("not behind agent-launcher confine"), "{message}");

    let no_separator = ProcessCommand::new("/opt/agent-launcher", &["confine", "--read", "/run/work"]);
    let (_, message) = refusal(translate(RunClient::Opencode, &no_separator, ROUTE_REMOTE));
    assert!(message.contains("not behind agent-launcher confine"), "{message}");

    // Each refusal below needs a written mount, which sbx needs as its primary workspace, so the gap and route
    // refusals are the ones reached. A plan with only read mounts is refused first, whatever else it holds.
    let read_only_only = ProcessCommand::new("/opt/agent-launcher", &["confine", "--read", "/run/work", "--keychain", "--", "opencode"]);
    let (code, message) = refusal(translate(RunClient::Opencode, &read_only_only, ROUTE_REMOTE));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.starts_with("opencode: the sbx plan has no written mount"), "{message}");

    let keychain = ProcessCommand::new("/opt/agent-launcher", &["confine", "--write", "/run/export", "--keychain", "--", "opencode"]);
    let (code, message) = refusal(translate(RunClient::Opencode, &keychain, ROUTE_REMOTE));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.starts_with("opencode: the sbx plan holds no way to give keychain:"), "{message}");

    let unknown = ProcessCommand::new("/opt/agent-launcher", &["confine", "--write", "/run/export", "--mystery", "x", "--", "opencode"]);
    assert!(refusal(translate(RunClient::Opencode, &unknown, ROUTE_REMOTE)).1.contains("--mystery x"));

    let confined = ProcessCommand::new("/opt/agent-launcher", &["confine", "--write", "/run/export", "--", "opencode"]);
    for route in ["{}", "not json", r#"{"baseURL": 3}"#] {
        assert_eq!(refusal(translate(RunClient::Opencode, &confined, route)).1, "the route names no base URL; the sbx backend allows only the LiteLLM proxy the route names");
    }
    assert!(refusal(translate(RunClient::Opencode, &confined, r#"{"baseURL":"ftp://x:21"}"#)).1.contains("names no host the sbx policy can allow"));
    assert!(refusal(translate(RunClient::Opencode, &confined, r#"{"baseURL":"http://:4000/v1"}"#)).1.contains("names no host"));
    // A base URL nested in lists and objects is found; a route with none at any depth is refused.
    assert!(translate(RunClient::Opencode, &confined, r#"{"a":[{"b":{"baseUrl":"http://x:9/v1"}}]}"#).is_ok());
    assert!(refusal(translate(RunClient::Opencode, &confined, r#"{"a":[1,{"b":true}]}"#)).1.contains("names no base URL"));
}

#[test]
fn a_loopback_route_allows_both_names_and_its_config_is_rewritten_to_the_one_the_sandbox_sees() {
    let local = r#"{"x":{"baseUrl":"http://127.0.0.1:4000/v1"}}"#;
    let plain = ProcessCommand::new("/opt/agent-launcher", &["confine", "--write", "/run/export", "--", "pi"]);
    let sbx = sbx_launch::translate(RunClient::Pi, "mg-test", &plain, local, &|_| Ok(String::new())).expect("a translation");
    assert_eq!(sbx.allow, ["host.docker.internal:4000", "localhost:4000"]);
    assert_eq!(sbx.policy_args().join(" "), "policy allow network --sandbox mg-test host.docker.internal:4000,localhost:4000");
    assert_eq!(sbx_launch::rewritten_config(RunClient::Pi, local).as_deref(), Some(r#"{"x":{"baseUrl":"http://host.docker.internal:4000/v1"}}"#));
    assert_eq!(sbx_launch::rewritten_config(RunClient::Pi, ROUTE_REMOTE), None, "a remote host needs no rewrite");
    assert_eq!(sbx_launch::rewritten_config(RunClient::Pi, "not json"), None);
    let remote = sbx_launch::translate(RunClient::Pi, "mg-test", &plain, ROUTE_REMOTE, &|_| Ok(String::new())).expect("a translation");
    assert_eq!(remote.allow, ["litellm.example.com:443"]);
}

#[test]
fn the_route_config_is_the_one_the_client_reads_and_each_client_has_its_own_name() {
    assert_eq!(sbx_launch::config_path(RunClient::Pi, "/run/export/pi-agent"), "/run/export/pi-agent/models.json");
    assert_eq!(sbx_launch::config_path(RunClient::Opencode, "/run/export.client-home"), "/run/export.client-home/opencode.json");
    assert!(sbx_launch::sandbox_name().starts_with("mg-"));
    assert_ne!(sbx_launch::sandbox_name(), String::new());
}

#[test]
fn an_opencode_route_with_no_litellm_base_url_is_refused_under_sbx_and_its_home_is_removed() {
    // A provider the owner's config does not declare has no base URL the sandbox's one rule could name.
    let disk = Disk::default();
    let sbx = Sbx::new(healthy);
    let env = sbx_env();
    let mut request = pi_request(RunClient::Opencode, Some(RunMode::ReadOnly));
    request.launch.model = "opencode-go/kimi".to_string();
    request.launch.provider = None;
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&request).expect_err("refused"));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.contains("names no base URL"), "{message}");
    assert!(!sbx.calls().iter().any(|call| call.starts_with("sbx create")), "nothing created");
    assert!(disk.removed.lock().expect("removed").contains(&"/run/export.client-home".to_string()));
}

#[test]
fn the_written_mounts_come_first_in_the_create_args_and_each_group_keeps_its_order() {
    let command = ProcessCommand::new("/opt/agent-launcher", &["confine", "--read", "/run/a", "--write", "/run/b", "--read", "/run/c", "--write", "/run/d", "--", "pi"]);
    let sbx = sbx_launch::translate(RunClient::Pi, "mg-test", &command, ROUTE_REMOTE, &|_| Ok(String::new())).expect("a translation");
    let pinned = SbxLaunch { image: PI_IMAGE.to_string(), ..sbx };
    assert_eq!(pinned.create_args(), ["create", "--name", "mg-test", "--no-input", "--pull", "missing", "-t", PI_IMAGE, "shell", "/run/b", "/run/d", "/run/a:ro", "/run/c:ro"]);
}

#[test]
fn a_plan_with_no_written_mount_is_refused_before_sbx_is_asked_for_anything() {
    // `translate` takes no process at all, so a refused plan cannot reach `sbx create`.
    let read_only = ProcessCommand::new("/opt/agent-launcher", &["confine", "--read", "/run/work", "--", "pi"]);
    let error = sbx_launch::translate(RunClient::Pi, "mg-test", &read_only, ROUTE_REMOTE, &|_| Ok(String::new())).expect_err("refused");
    assert_eq!(error.code, "sandbox-unsupported");
    assert_eq!(error.message, "pi: the sbx plan has no written mount, and sbx needs one as its primary workspace");
}

#[test]
fn a_create_that_failed_leaves_no_removal_warning_and_a_removal_after_a_create_still_warns() {
    let command = ProcessCommand::new("/opt/agent-launcher", &["confine", "--write", "/run/export", "--", "pi"]);
    let sbx = sbx_launch::translate(RunClient::Pi, "mg-test", &command, ROUTE_REMOTE, &|_| Ok(String::new())).expect("a translation");

    // `sbx create` refused: nothing was made, so a removal that exits or does not run says nothing.
    let create_refused_rm_exits = Sbx::new(|args| match args[0] {
        "create" => answer("", "no room\n", 1),
        "rm" => answer("", "busy", 1),
        _ => healthy(args),
    });
    let create_refused_rm_unrun = Sbx::new(|args| match args[0] {
        "create" => answer("", "no room\n", 1),
        "rm" => Err("sbx rm did not run".to_string()),
        _ => healthy(args),
    });
    for process in [&create_refused_rm_exits, &create_refused_rm_unrun] {
        let run = sbx_launch::run(process, &Disk::default(), &sbx, &command, &Notes::default());
        assert!(run.ran.is_err(), "the create was refused");
        assert!(run.warnings.is_empty(), "{:?}", run.warnings);
    }

    // `sbx create` made the sandbox: a removal that exits or does not run is a warning naming it.
    let rm_exits = Sbx::new(|args| if args[0] == "rm" { answer("", "busy", 1) } else { healthy(args) });
    let rm_unrun = Sbx::new(|args| if args[0] == "rm" { Err("sbx rm did not run".to_string()) } else { healthy(args) });
    for process in [&rm_exits, &rm_unrun] {
        let run = sbx_launch::run(process, &Disk::default(), &sbx, &command, &Notes::default());
        assert!(run.ran.is_ok(), "the create and the exec went through");
        assert_eq!(run.warnings.len(), 1, "{:?}", run.warnings);
        assert!(run.warnings[0].contains("sbx rm --force mg-test"), "{:?}", run.warnings);
    }
}

#[test]
fn a_loopback_route_whose_config_cannot_be_read_back_refuses_the_sbx_launch_after_its_directory_is_removed() {
    let disk = Disk { unreadable: Some("/run/export/pi-agent/models.json".to_string()), ..pi_disk("http://localhost:4000/v1") };
    let sbx = Sbx::new(healthy);
    let env = sbx_env();
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly))).expect_err("refused"));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.contains("the route's config /run/export/pi-agent/models.json cannot be read"), "{message}");
    assert!(!sbx.calls().iter().any(|call| call.starts_with("sbx create")), "no sandbox is named");
    assert!(disk.removed.lock().expect("removed").iter().any(|dir| dir == "/run/export/pi-agent"), "the agent directory goes before the refusal");
}

#[test]
fn a_loopback_rewrite_that_cannot_be_written_refuses_the_sbx_launch_before_sbx_creates_anything() {
    let disk = Disk { refuse_rewrite: true, ..pi_disk("http://localhost:4000/v1") };
    let sbx = Sbx::new(healthy);
    let env = sbx_env();
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly))).expect_err("refused"));
    assert_eq!(code, "unexpected", "the write's own error, as from_fs names it");
    assert!(message.contains("EACCES: /run/export/pi-agent/models.json"), "{message}");
    assert!(!sbx.calls().iter().any(|call| call.starts_with("sbx create")), "no sandbox is named");
}

#[test]
fn a_remote_route_is_allowed_by_its_own_host_and_its_config_is_left_as_it_is() {
    let disk = pi_disk("https://litellm.example.com/v1");
    let sbx = Sbx::new(healthy);
    let env = sbx_env();
    let outcome = ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Pi, Some(RunMode::ReadOnly))).expect("a run");
    assert_eq!(outcome.result.stdout, "ok\n");
    let calls = sbx.calls();
    assert!(calls[4].ends_with(" litellm.example.com:443"), "{calls:?}");
    assert!(disk.files.lock().expect("files").get("/run/export/pi-agent/models.json").expect("the agent directory's models").contains("https://litellm.example.com/v1"));
}

#[test]
fn a_client_the_backend_does_not_run_and_an_sbx_call_that_did_not_run_are_named() {
    let plain = ProcessCommand::new("/opt/agent-launcher", &["confine", "--write", "/run/export", "--", "pi"]);
    let refused = sbx_launch::translate(RunClient::Codex, "mg-test", &plain, ROUTE_REMOTE, &|_| Ok(String::new())).expect_err("codex has no sandbox program");
    assert_eq!(refused.code, "sandbox-unsupported");
    assert_eq!(refused.message, "codex: no sbx program");

    let sbx = sbx_launch::translate(RunClient::Pi, "mg-test", &plain, ROUTE_REMOTE, &|_| Ok(String::new())).expect("a translation");
    let process = Sbx::new(|args| if args.first() == Some(&"create") { Err("the binary is missing".to_string()) } else { healthy(args) });
    let run = sbx_launch::run(&process, &Disk::default(), &sbx, &plain, &Notes::default());
    assert_eq!(run.ran.err().as_deref(), Some("sbx run failed: sbx create did not run: the binary is missing"));
    assert!(run.warnings.is_empty(), "a sandbox sbx never made leaves nothing to warn about");
}

#[test]
fn an_opencode_keyed_run_reads_its_key_from_the_secret_store_and_allows_the_remote_route() {
    let disk = Disk::default();
    disk.files.lock().expect("files").insert("/home/me/.config/opencode/opencode.json".to_string(), ROUTE_REMOTE.replace("litellm-x", "litellm-vibe"));
    let sbx = Sbx::new(|args| if args.first() == Some(&"get") { answer("value\n", "", 0) } else { healthy(args) });
    let env = sbx_env();
    let mut request = pi_request(RunClient::Opencode, Some(RunMode::ReadOnly));
    request.launch.model = "litellm-vibe/glm-5.3".to_string();
    request.launch.provider = None;
    let outcome = ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&request).expect("a run");
    assert_eq!(outcome.result.stdout, "ok\n");
    let calls = sbx.calls();
    assert!(calls.contains(&"agent-kit-secret get LITELLM_VIBE_KEY".to_string()), "{calls:?}");
    assert!(calls.iter().any(|call| call.starts_with("sbx policy allow network ") && call.ends_with(" litellm.example.com:443")), "{calls:?}");
}

/// The opencode launch whose key is read from the store, translated with its sbx env file in the launch's own directory.
fn keyed_launch() -> SbxLaunch {
    let sbx = sbx_launch::translate(RunClient::Opencode, "mg-test", &keyed_opencode(true), ROUTE_REMOTE, &|_| Ok("secret-value".to_string())).expect("a translation");
    SbxLaunch { image: OPENCODE_IMAGE.to_string(), env_file: "/run/export.client-home/sbx.env".to_string(), ..sbx }
}

#[test]
fn the_env_file_holds_the_values_for_one_exec_and_is_removed_after_it_and_no_value_reaches_argv() {
    let process = Sbx::new(healthy);
    let disk = Disk::default();
    let run = sbx_launch::run(&process, &disk, &keyed_launch(), &keyed_opencode(true), &Notes::default());
    assert!(run.ran.is_ok(), "{:?}", run.ran);
    assert!(run.warnings.is_empty(), "{:?}", run.warnings);
    assert_eq!(disk.files.lock().expect("files").get("/run/export.client-home/sbx.env").expect("written"), "LITELLM_KEY=secret-value\nXDG_CONFIG_HOME=/run/export.client-home/.config\n");
    assert!(disk.removed.lock().expect("removed").contains(&"/run/export.client-home/sbx.env".to_string()));
    let calls = process.calls();
    assert!(calls.iter().any(|call| call.starts_with("sbx exec -i -w /run/work --env-file /run/export.client-home/sbx.env mg-test opencode")), "{calls:?}");
    assert!(calls.iter().all(|call| !call.contains("secret-value")), "a value is in argv: {calls:?}");
}

#[test]
fn an_env_file_is_removed_even_when_the_exec_fails_and_one_that_cannot_be_written_or_removed_is_named() {
    let failing = Sbx { run: Box::new(|_| Err("the program could not start".to_string())), ..Sbx::new(healthy) };
    let disk = Disk::default();
    let run = sbx_launch::run(&failing, &disk, &keyed_launch(), &keyed_opencode(true), &Notes::default());
    assert_eq!(run.ran.err().as_deref(), Some("sbx run failed: the program could not start"));
    assert!(disk.removed.lock().expect("removed").contains(&"/run/export.client-home/sbx.env".to_string()));

    // Refused to write: nothing is exec'd, and the sandbox is still removed.
    let process = Sbx::new(healthy);
    let refusing = Disk { refuse_env_file: true, ..Disk::default() };
    let run = sbx_launch::run(&process, &refusing, &keyed_launch(), &keyed_opencode(true), &Notes::default());
    assert_eq!(run.ran.err().as_deref(), Some("sbx run failed: the sandbox's env file was not written: EACCES: /run/export.client-home/sbx.env"));
    let calls = process.calls();
    assert!(!calls.iter().any(|call| call.starts_with("sbx exec")), "{calls:?}");
    assert!(calls.last().expect("calls").starts_with("sbx rm --force mg-test"), "{calls:?}");

    // Refused to remove: a warning the outcome carries, naming the file.
    let stuck = Disk { refuse_env_remove: true, ..Disk::default() };
    let run = sbx_launch::run(&Sbx::new(healthy), &stuck, &keyed_launch(), &keyed_opencode(true), &Notes::default());
    assert!(run.ran.is_ok());
    assert_eq!(run.warnings, ["the sandbox's env file /run/export.client-home/sbx.env was not removed: EBUSY: /run/export.client-home/sbx.env"]);
}

/// A Pi write run's request: its worktree written, the git directories its writes need written, and the rest of
/// its git read.
fn write_request() -> ClientRunRequest {
    let mut request = pi_request(RunClient::Pi, Some(RunMode::Write));
    request.paths.work_dir = "/repo/.git/model-gateway/worktrees/r1".to_string();
    request.paths.git_writable = Some(vec!["/repo/.git/worktrees/r1".to_string(), "/repo/.git/objects".to_string(), "/repo/.git/refs".to_string()]);
    request.paths.git_view.reads = vec!["/repo/.git/HEAD".to_string(), "/repo/.git/packed-refs".to_string()];
    request
}

#[test]
fn a_write_run_mounts_its_worktree_and_git_directories_written_and_never_the_main_checkout() {
    let disk = pi_disk("http://localhost:4000/v1");
    let sbx = Sbx::new(healthy);
    let env = sbx_env();
    ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&write_request()).expect("a write run");
    let calls = sbx.calls();
    let create = calls.iter().find(|call| call.starts_with("sbx create ")).expect("a create call");
    let mounts: Vec<&str> = create.split(" shell ").nth(1).expect("mounts").split_whitespace().collect();
    let written: Vec<&str> = mounts.iter().copied().filter(|mount| !mount.ends_with(":ro")).collect();
    assert_eq!(written[0], "/repo/.git/model-gateway/worktrees/r1", "the worktree is the primary workspace: {mounts:?}");
    for dir in ["/repo/.git/worktrees/r1", "/repo/.git/objects", "/repo/.git/refs"] {
        assert!(written.contains(&dir), "{dir} is written: {mounts:?}");
    }
    for read in ["/repo/.git/HEAD:ro", "/repo/.git/packed-refs:ro"] {
        assert!(mounts.contains(&read), "{read} is read-only: {mounts:?}");
    }
    // Neither the repository's checkout nor its whole git directory is mounted, and the one written path outside the worktree is a git directory.
    assert!(!mounts.iter().any(|mount| mount.trim_end_matches(":ro") == "/repo" || mount.trim_end_matches(":ro") == "/repo/.git"), "{mounts:?}");
    assert!(written.iter().all(|mount| mount.starts_with("/repo/.git/") || mount.starts_with("/run/export")), "{written:?}");
    assert!(calls.last().expect("calls").starts_with("sbx rm --force mg-"), "{calls:?}");
}

#[test]
fn a_cancelled_run_and_a_stall_kill_remove_the_sandbox_and_the_env_file() {
    for (ending, timed_out) in [(Ending { signal: None, cancelled_by: Some(15), pid: Some(7) }, false), (Ending { signal: Some(15), cancelled_by: None, pid: Some(7) }, true)] {
        let process = Sbx { end: Some((ending, timed_out)), ..Sbx::new(healthy) };
        let disk = Disk::default();
        let run = sbx_launch::run(&process, &disk, &keyed_launch(), &keyed_opencode(true), &Notes::default());
        assert!(run.ran.is_ok(), "{:?}", run.ran);
        assert!(run.warnings.is_empty(), "{:?}", run.warnings);
        assert_eq!(process.calls().last().map(String::as_str), Some("sbx rm --force mg-test"));
        assert!(disk.removed.lock().expect("removed").contains(&"/run/export.client-home/sbx.env".to_string()), "the env file outlived the run");
    }
}

#[test]
fn recovery_removes_the_sandboxes_of_ended_gateways_and_keeps_live_and_foreign_ones() {
    let own = std::process::id();
    let listing = format!("mg-100-aa\nmg-200-bb\nmg-{own}-cc\nmg-stuck-dd\nother-box\nmg-300-ee\n");
    let process = Sbx { alive: vec![200], ..Sbx::new(move |args| if args[0] == "ls" { answer(&listing, "", 0) } else if args.get(2) == Some(&"mg-300-ee") { answer("", "busy", 1) } else { healthy(args) }) };
    let warnings = sbx_launch::prune_orphans(&process);
    assert_eq!(process.calls(), ["sbx ls -q", "sbx rm --force mg-100-aa", "sbx rm --force mg-300-ee"]);
    assert_eq!(warnings, ["the sandbox mg-300-ee of an ended run was not removed; remove it with sbx rm --force mg-300-ee"]);

    // A listing that cannot be had prunes nothing and says nothing.
    for listing in [answer("", "no daemon", 1), Err("sbx missing".to_string())] {
        let process = Sbx::new(move |_| listing.clone());
        assert!(sbx_launch::prune_orphans(&process).is_empty());
        assert_eq!(process.calls(), ["sbx ls -q"]);
    }
}

const VIBE_CONFIG: &str = r#"active_model = "mistral-medium"
[[providers]]
name = "mistral"
api_base = "https://api.mistral.ai/v1"
api_key_env_var = "MISTRAL_API_KEY"
backend = "mistral"

[[providers]]
name = "litellm-vibe"
api_base = "http://localhost:4000/v1"
api_key_env_var = "LITELLM_VIBE_KEY"
backend = "generic"

[[models]]
name = "devstral-small"
provider = "mistral"
alias = "devstral-small"

[[models]]
name = "glm-5.3"
provider = "litellm-vibe"
alias = "litellm-vibe/glm-5.3"
"#;

/// A Vibe launch as `launchers` builds it for a confined run: no wrapper, the key named by `--env`.
fn confined_vibe(model: &str) -> ProcessCommand {
    let words = ["confine", "--read", "/run/work", "--write", "/run/export", "--write", "/run/export.client-home", "--net-connect", "443", "--env", "MISTRAL_API_KEY", "--env", "VIBE_HOME", "--", "vibe", "--prompt-file", "/run/prompt.md", "--model", model, "--agent", "delegate"];
    ProcessCommand {
        cwd: Some("/run/work".to_string()),
        env: [("MISTRAL_API_KEY".to_string(), "mistral-secret".to_string()), ("VIBE_HOME".to_string(), "/run/export.client-home/vibe".to_string())].into_iter().collect(),
        ..ProcessCommand::new("/opt/agent-launcher", &words)
    }
}

#[test]
fn a_direct_mistral_vibe_launch_allows_api_mistral_ai_and_passes_its_key_by_name_only() {
    let sbx = sbx_launch::translate(RunClient::Vibe, "mg-test", &confined_vibe("devstral-small"), VIBE_CONFIG, &|_| Ok(String::new())).expect("a translation");
    assert_eq!(sbx.program, "vibe");
    assert_eq!(sbx.allow, ["api.mistral.ai:443"]);
    assert_eq!(sbx.env, ["MISTRAL_API_KEY=mistral-secret", "VIBE_HOME=/run/export.client-home/vibe"]);
    let pinned = SbxLaunch { image: VIBE_IMAGE.to_string(), env_file: "/run/export.client-home/sbx.env".to_string(), ..sbx };
    assert_eq!(pinned.create_args(), ["create", "--name", "mg-test", "--no-input", "--pull", "missing", "-t", VIBE_IMAGE, "shell", "/run/export", "/run/export.client-home", "/run/work:ro"]);
    let exec = pinned.exec_args();
    assert!(exec.windows(2).any(|pair| pair == ["--env-file", "/run/export.client-home/sbx.env"]), "{exec:?}");
    assert!(exec.iter().all(|word| !word.contains("mistral-secret")), "no value in argv: {exec:?}");
    assert_eq!(exec.iter().skip_while(|word| *word != "mg-test").nth(1).map(String::as_str), Some("vibe"));
}

#[test]
fn a_litellm_routed_vibe_launch_allows_the_proxy_by_both_loopback_names_and_a_config_without_models_goes_direct() {
    let sbx = sbx_launch::translate(RunClient::Vibe, "mg-test", &confined_vibe("litellm-vibe/glm-5.3"), VIBE_CONFIG, &|_| Ok(String::new())).expect("a translation");
    assert_eq!(sbx.allow, ["host.docker.internal:4000", "localhost:4000"]);
    let rewritten = sbx_launch::rewritten_config(RunClient::Vibe, VIBE_CONFIG).expect("a rewrite");
    assert!(rewritten.contains("api_base = \"http://host.docker.internal:4000/v1\"") && rewritten.contains("https://api.mistral.ai/v1"), "{rewritten}");
    assert_eq!(sbx_launch::rewritten_config(RunClient::Vibe, "[[providers]]\nname = \"m\"\nbase = 1\n[[providers]]\nname = \"r\"\napi_base = \"https://x.example/v1\"\n"), None);
    assert_eq!(sbx_launch::rewritten_config(RunClient::Vibe, "= not toml"), None);
    assert_eq!(sbx_launch::config_path(RunClient::Vibe, "/run/home"), "/run/home/vibe/config.toml");
    // No config, an unknown model, a provider without a base and a model-less launch are all Mistral's own.
    for config in ["", "models = 1\nproviders = 1", "[[models]]\nname = \"devstral-small\"\nprovider = \"gone\"\n"] {
        let sbx = sbx_launch::translate(RunClient::Vibe, "mg-test", &confined_vibe("devstral-small"), config, &|_| Ok(String::new())).expect("a translation");
        assert_eq!(sbx.allow, ["api.mistral.ai:443"], "{config}");
    }
    let agent_picks = ProcessCommand::new("/opt/agent-launcher", &["confine", "--write", "/run/export", "--", "vibe", "--agent", "a"]);
    assert_eq!(sbx_launch::translate(RunClient::Vibe, "mg-test", &agent_picks, VIBE_CONFIG, &|_| Ok(String::new())).expect("a translation").allow, ["api.mistral.ai:443"]);
    let broken = sbx_launch::translate(RunClient::Vibe, "mg-test", &confined_vibe("x"), "[[models", &|_| Ok(String::new())).expect_err("refused");
    assert_eq!(broken.code, "sandbox-unsupported");
    assert!(broken.message.starts_with("vibe: the run home's config.toml does not parse"), "{}", broken.message);
}

#[test]
fn a_vibe_login_only_in_the_keychain_is_refused_naming_the_keychain_and_a_key_read_outside_runs_in_the_vibe_image() {
    let request = |provider: &str| {
        let mut request = pi_request(RunClient::Vibe, Some(RunMode::ReadOnly));
        request.launch.model = "devstral-small".to_string();
        request.launch.provider = Some(provider.to_string());
        request.paths.run_dir = Some("/run".to_string());
        request
    };
    let owner_disk = || {
        let disk = Disk::default();
        disk.files.lock().expect("files").insert("/home/me/.vibe/config.toml".to_string(), VIBE_CONFIG.to_string());
        disk
    };
    let (disk, kept) = (owner_disk(), owner_disk());
    let env = sbx_env();
    // The keychain holds nothing the gateway can read: the launch would need `--keychain`, which no sandbox reaches.
    let none = Sbx::new(|args| if args.first() == Some(&"find-generic-password") { answer("", "", 44) } else { healthy(args) });
    let (code, message) = gateway_code(ClientRun { process: &none, files: &disk, clock: &Fixed, env: &env }.launch(&request("mistral")).expect_err("refused"));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.starts_with("vibe: the sbx plan holds no way to give keychain:"), "{message}");
    assert!(!none.calls().iter().any(|call| call.starts_with("sbx create")), "nothing was made");

    // The key read outside goes in by name, and the sandbox starts from the Vibe image with Mistral allowed.
    let keyed = Sbx::new(|args| if args.first() == Some(&"find-generic-password") { answer("mistral-secret\n", "", 0) } else { healthy(args) });
    let outcome = ClientRun { process: &keyed, files: &kept, clock: &Fixed, env: &env }.launch(&request("mistral")).expect("a run");
    assert_eq!(outcome.result.stdout, "ok\n");
    let calls = keyed.calls();
    assert!(calls.iter().any(|call| call.starts_with("sbx create ") && call.contains(&format!("-t {VIBE_IMAGE} "))), "{calls:?}");
    assert!(calls.iter().any(|call| call.starts_with("sbx policy allow network ") && call.ends_with(" api.mistral.ai:443")), "{calls:?}");
    assert!(calls.iter().any(|call| call.starts_with("sbx exec ") && call.contains(" vibe ") && !call.contains("mistral-secret")), "{calls:?}");
    assert!(calls.last().is_some_and(|call| call.starts_with("sbx rm --force mg-")), "{calls:?}");
}
