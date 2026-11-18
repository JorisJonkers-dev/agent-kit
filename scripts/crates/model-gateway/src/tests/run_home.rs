//! The per-run home (#903): what a launch of OpenCode or Hermes is given in place of the owner's home, the
//! OpenCode config that makes read-only mean read-only tools, and what each client's launch does about MCP. Every
//! client is a fake; nothing here starts a program.

use crate::client_run::{ClientRun, ClientRunError, ClientRunRequest};
use crate::extras::RunExtras;
use crate::json::{self, Value};
use crate::launchers::{LaunchPaths, LaunchRequest, VIBE_ENV};
use crate::ports::{Files, FsError, Process, ProcessCommand, ProcessResult};
use crate::run::{RunClient, RunMode};
use crate::run_home::{opencode_config, prepare_hermes, prepare_opencode, run_home_dir};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// Files in memory: what a test puts there, what a launch writes privately, and what it removes.
#[derive(Default)]
struct Disk {
    files: Mutex<BTreeMap<String, String>>,
    removed: Mutex<Vec<String>>,
    /// A private write to a path with this in it fails.
    write_fails: Option<&'static str>,
}

impl Disk {
    fn with(self, path: &str, text: &str) -> Disk {
        self.files.lock().expect("files").insert(path.to_string(), text.to_string());
        self
    }

    fn text(&self, path: &str) -> Option<String> {
        self.files.lock().expect("files").get(path).cloned()
    }
}

fn nothing() -> FsError {
    FsError { code: "EACCES".into(), message: "EACCES: denied".into() }
}

impl Files for Disk {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        self.text(path).ok_or(FsError { code: "ENOENT".into(), message: format!("ENOENT: {path}") })
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
        Ok(self.text(path).is_some())
    }
    fn append(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn stamp(&self, _path: &str) -> Option<crate::ports::FileStamp> {
        None
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        if self.write_fails.is_some_and(|part| path.contains(part)) {
            return Err(nothing());
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

/// A process that records every launch and answers with what a test says.
struct Programs {
    launched: Result<ProcessResult, String>,
    ran: Mutex<Vec<ProcessCommand>>,
}

impl Programs {
    fn answering() -> Programs {
        Programs { launched: Ok(ProcessResult { stdout: String::new(), stderr: String::new(), exit_code: 0, timed_out: false }), ran: Mutex::new(Vec::new()) }
    }
    fn first(&self) -> ProcessCommand {
        self.ran.lock().expect("ran")[0].clone()
    }
}

impl Process for Programs {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Err("a launch goes through run".into())
    }
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.ran.lock().expect("ran").push(command.clone());
        self.launched.clone()
    }
}

fn env() -> HashMap<String, String> {
    HashMap::from([("HOME".to_string(), "/home/me".to_string())])
}

/// The owner's OpenCode config: an MCP server of their own, a LiteLLM provider with two models, a second provider.
const OWNER_OPENCODE: &str = r#"{"mcp":{"servers":{"memory-api":{"type":"remote","url":"https://example.invalid/mcp"},"outline":{"type":"local","command":["x"]}}},
"provider":{"litellm-vibe":{"npm":"@ai-sdk/openai-compatible","options":{"apiKey":"{env:LITELLM_VIBE_KEY}","baseURL":"https://litellm.example/v1"},"models":{"glm-5.3":{"name":"glm"},"other":{}}},
"litellm-list":{"models":["x"]},"litellm-free":{"options":{}}}}"#;

const OWNER_HERMES: &str = "model:\n  default: x\nmcp_servers:\n  olcli:\n    command: olcli-mcp\nproviders:\n  litellm-vibe:\n    name: litellm-vibe\n    key_env: LITELLM_VIBE_KEY\n\n    models:\n      glm-5.3: {}\n  litellm-free:\n    name: litellm-free\nother: 1\n";

fn owner() -> Disk {
    Disk::default().with("/home/me/.config/opencode/opencode.json", OWNER_OPENCODE).with("/home/me/.hermes/config.yaml", OWNER_HERMES)
}

fn object<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("no {key}"))
}

/// The `(action, effect)` of each `permissions` rule, in order.
fn rules(config: &Value) -> Vec<(String, String)> {
    let Value::Arr(rules) = object(config, "permissions") else { panic!("no rules") };
    rules.iter().map(|rule| (object(rule, "action").as_str().unwrap().to_string(), object(rule, "effect").as_str().unwrap().to_string())).collect()
}

/// What a rule list lets an action do: the effect of the last rule that names it, else of the `*` rule.
fn effect(rules: &[(String, String)], action: &str) -> String {
    rules.iter().rev().find(|(name, _)| name == action).or_else(|| rules.iter().rev().find(|(name, _)| name == "*")).map(|(_, effect)| effect.clone()).unwrap()
}

#[test]
fn a_read_only_opencode_run_gets_read_tools_and_nothing_that_acts_outside_its_directory() {
    let config = opencode_config(Some(RunMode::ReadOnly), None);
    let rules = rules(&config);
    for action in ["read", "grep", "glob", "webfetch", "websearch"] {
        assert_eq!(effect(&rules, action), "allow", "{action}");
    }
    for action in ["shell", "execute", "subagent", "external_directory", "edit", "skill", "an_mcp_server_tool"] {
        assert_eq!(effect(&rules, action), "deny", "{action}");
    }
    // The v1 keys say the same for an OpenCode that reads them: no shell, no write, edit or patch.
    let tools = object(&config, "tools");
    for tool in ["bash", "task", "write", "edit", "patch"] {
        assert_eq!(object(tools, tool), &Value::Bool(false), "{tool}");
    }
    let permission = object(&config, "permission");
    assert_eq!((object(permission, "bash"), object(permission, "edit")), (&Value::str("deny"), &Value::str("deny")));
    assert_eq!(object(permission, "external_directory"), &Value::str("deny"));
    // No MCP server and no provider of the owner's: the config declares none.
    assert!(config.as_object().unwrap().get("mcp").is_none() && config.as_object().unwrap().get("provider").is_none());
}

#[test]
fn a_writing_opencode_run_keeps_its_edit_tools_but_never_a_shell_or_a_path_outside_its_directory() {
    let config = opencode_config(Some(RunMode::Write), None);
    let rules = rules(&config);
    assert_eq!(effect(&rules, "edit"), "allow");
    for action in ["shell", "execute", "subagent", "external_directory"] {
        assert_eq!(effect(&rules, action), "deny", "{action}");
    }
    let tools = object(&config, "tools");
    assert_eq!((object(tools, "bash"), object(tools, "task")), (&Value::Bool(false), &Value::Bool(false)));
    assert!(tools.as_object().unwrap().get("edit").is_none());
    assert_eq!(object(object(&config, "permission"), "edit"), &Value::str("allow"));
    // A run that names no mode is read-only.
    assert_eq!(effect(&super::run_home::rules(&opencode_config(None, None)), "edit"), "deny");
}

#[test]
fn the_deny_rules_come_last_so_the_last_match_that_wins_is_a_deny() {
    let rules = rules(&opencode_config(Some(RunMode::ReadOnly), None));
    assert_eq!(rules[0], ("*".to_string(), "deny".to_string()));
    let last_allow = rules.iter().rposition(|(_, effect)| effect == "allow").unwrap();
    let first_deny_of_action = rules.iter().position(|(action, effect)| action == "shell" && effect == "deny").unwrap();
    assert!(first_deny_of_action > last_allow);
}

#[test]
fn an_opencode_home_holds_the_one_provider_the_model_names_and_none_of_the_owners_mcp_servers() {
    let disk = owner();
    let home = prepare_opencode(&disk, &env(), "litellm-vibe/glm-5.3", Some(RunMode::ReadOnly), "/run/h").expect("a home");
    let config = json::parse(&disk.text("/run/h/opencode.json").expect("config")).expect("JSON");
    let providers = object(&config, "provider");
    assert_eq!(providers.as_object().unwrap().entries().iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["litellm-vibe"]);
    // The model the run names is the only one kept, and the key stays a reference the wrapper fills.
    let entry = object(providers, "litellm-vibe");
    assert_eq!(object(entry, "models").as_object().unwrap().entries().iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["glm-5.3"]);
    assert_eq!(object(object(entry, "options"), "apiKey"), &Value::str("{env:LITELLM_VIBE_KEY}"));
    let text = disk.text("/run/h/opencode.json").unwrap();
    assert!(!text.contains("memory-api") && !text.contains("outline") && !text.contains("example.invalid/mcp"), "{text}");
    // Nothing of the owner's other files is written, and no sign-in for a LiteLLM provider.
    assert_eq!(disk.files.lock().unwrap().keys().filter(|path| path.starts_with("/run/h")).collect::<Vec<_>>(), ["/run/h/opencode.json"]);

    // Every variable points inside the home, and the config is named by its own variable.
    for name in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "OPENCODE_CONFIG", "OPENCODE_CONFIG_DIR"] {
        assert!(home.env[name].starts_with("/run/h"), "{name}={}", home.env[name]);
    }
    assert_eq!(home.env["OPENCODE_CONFIG"], "/run/h/opencode.json");
    for (name, value) in [("OPENCODE_DISABLE_PROJECT_CONFIG", "1"), ("OPENCODE_DISABLE_CLAUDE_CODE", "1"), ("OPENCODE_DISABLE_AUTOUPDATE", "1"), ("OPENCODE_CONFIG_CONTENT", "{}")] {
        assert_eq!(home.env[name], value);
    }
}

#[test]
fn an_opencode_provider_of_its_own_keeps_the_owners_data_directory_because_its_sign_in_is_there() {
    let disk = owner();
    let home = prepare_opencode(&disk, &env(), "opencode-go/kimi", Some(RunMode::Write), "/run/h").expect("a home");
    // The sign-in is in the owner's `opencode.db`, so the data directory is the owner's; everything else is the run's.
    assert_eq!(home.env["XDG_DATA_HOME"], "/home/me/.local/share");
    for name in ["HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"] {
        assert!(home.env[name].starts_with("/run/h"), "{name}={}", home.env[name]);
    }
    // Nothing of the owner's files is copied, and the built-in provider needs no provider entry.
    assert_eq!(disk.files.lock().unwrap().keys().filter(|path| path.starts_with("/run/h")).collect::<Vec<_>>(), ["/run/h/opencode.json"]);
    assert!(json::parse(&disk.text("/run/h/opencode.json").unwrap()).unwrap().as_object().unwrap().get("provider").is_none());
    // A LiteLLM provider's data directory is the run's own: its key comes from the secret store.
    let routed = prepare_opencode(&disk, &env(), "litellm-vibe/glm-5.3", None, "/run/h").expect("a home");
    assert_eq!(routed.env["XDG_DATA_HOME"], "/run/h/.local/share");
    prepare_opencode(&Disk::default(), &env(), "opencode/free-model", None, "/run/h").expect("a home");
    prepare_opencode(&owner(), &env(), "no-slash-model", None, "/run/h").expect("a model with no provider");
}

#[test]
fn an_opencode_home_follows_the_xdg_variables_of_the_owner_and_keeps_a_model_the_provider_does_not_list() {
    let xdg = HashMap::from([("XDG_CONFIG_HOME".to_string(), "/xdg/c".to_string()), ("XDG_DATA_HOME".to_string(), "/xdg/d".to_string()), ("HOME".to_string(), "/home/me".to_string())]);
    let disk = Disk::default().with("/xdg/c/opencode/opencode.json", OWNER_OPENCODE);
    prepare_opencode(&disk, &xdg, "litellm-vibe/unlisted", None, "/run/h").expect("a home");
    let config = json::parse(&disk.text("/run/h/opencode.json").unwrap()).unwrap();
    assert_eq!(object(object(object(&config, "provider"), "litellm-vibe"), "models").as_object().unwrap().entries().len(), 2);
    assert_eq!(prepare_opencode(&disk, &xdg, "opencode/free", None, "/run/h").expect("a home").env["XDG_DATA_HOME"], "/xdg/d");
    // A provider whose models are not a table is kept as it is.
    prepare_opencode(&Disk::default().with("/xdg/c/opencode/opencode.json", OWNER_OPENCODE), &xdg, "litellm-list/x", None, "/run/h").expect("a home");
    let empty = HashMap::new();
    let refused = prepare_opencode(&Disk::default(), &empty, "litellm-vibe/m", None, "/run/h").expect_err("no config");
    assert!(refused.contains("/.config/opencode/opencode.json cannot be read"), "{refused}");
}

#[test]
fn an_opencode_launch_the_owners_home_cannot_serve_is_refused_not_started_bare() {
    let missing = prepare_opencode(&Disk::default(), &env(), "litellm-vibe/glm-5.3", None, "/run/h").expect_err("no config");
    assert!(missing.contains("/home/me/.config/opencode/opencode.json cannot be read; re-run agent-kit setup"), "{missing}");
    let broken = Disk::default().with("/home/me/.config/opencode/opencode.json", "{");
    assert!(prepare_opencode(&broken, &env(), "litellm-vibe/glm-5.3", None, "/run/h").expect_err("broken").contains("cannot be read"));
    let none = prepare_opencode(&owner(), &env(), "litellm-nowhere/m", None, "/run/h").expect_err("no provider");
    assert!(none.contains("has no litellm-nowhere provider"), "{none}");
    let denied = Disk { write_fails: Some("opencode.json"), ..owner() };
    assert!(prepare_opencode(&denied, &env(), "litellm-vibe/glm-5.3", None, "/run/h").expect_err("denied").contains("cannot write the launch's home /run/h: EACCES"));
}

#[test]
fn a_hermes_home_holds_the_one_provider_the_run_names_and_no_mcp_server() {
    let disk = owner();
    let home = prepare_hermes(&disk, &env(), "litellm-vibe", "/run/h").expect("a home");
    let config = disk.text("/run/h/.hermes/config.yaml").expect("config");
    assert_eq!(
        config,
        "providers:\n  litellm-vibe:\n    name: litellm-vibe\n    key_env: LITELLM_VIBE_KEY\n\n    models:\n      glm-5.3: {}\nmcp_servers: {}\n"
    );
    assert!(!config.contains("olcli") && !config.contains("litellm-free"));
    assert_eq!(home.env["HERMES_HOME"], "/run/h/.hermes");
    assert_eq!((home.env["HOME"].as_str(), home.env["XDG_CONFIG_HOME"].as_str()), ("/run/h", "/run/h/.config"));
    // The last provider of the block ends at the next top-level key.
    let last = prepare_hermes(&owner(), &env(), "litellm-free", "/run/h").expect("a home");
    assert_eq!(last.dir, "/run/h");
    // HERMES_HOME names the owner's home when it is set.
    let named = HashMap::from([("HERMES_HOME".to_string(), "/h".to_string())]);
    prepare_hermes(&Disk::default().with("/h/config.yaml", OWNER_HERMES), &named, "litellm-vibe", "/run/h").expect("a home");
}

#[test]
fn a_hermes_launch_the_owners_home_cannot_serve_is_refused() {
    let missing = prepare_hermes(&Disk::default(), &env(), "litellm-vibe", "/run/h").expect_err("none");
    assert!(missing.contains("/home/me/.hermes/config.yaml cannot be read"), "{missing}");
    let nothing = prepare_hermes(&owner(), &env(), "litellm-nowhere", "/run/h").expect_err("none");
    assert!(nothing.contains("has no litellm-nowhere provider"), "{nothing}");
    let without = Disk::default().with("/home/me/.hermes/config.yaml", "model:\n  default: x\n");
    assert!(prepare_hermes(&without, &env(), "litellm-vibe", "/run/h").is_err());
    let denied = Disk { write_fails: Some("config.yaml"), ..owner() };
    assert!(prepare_hermes(&denied, &env(), "litellm-vibe", "/run/h").expect_err("denied").contains("cannot write the launch's home /run/h: EACCES"));
}

fn request(client: RunClient, model: &str, provider: Option<&str>, mode: RunMode) -> ClientRunRequest {
    ClientRunRequest {
        launch: LaunchRequest { model: model.into(), time_limit_s: 30.0, max_price_usd: None, mode: Some(mode), provider: provider.map(str::to_string) },
        binary: "client".into(),
        client,
        prompt: "ask".into(),
        paths: LaunchPaths { export_dir: "/run/export".into(), prompt_path: "/run/prompt.md".into(), run_dir: Some("/run".into()), work_dir: "/run/work".into(), git_writable: None },
        extras: RunExtras::default(),
    }
}

fn run(disk: &Disk, programs: &Programs, request: &ClientRunRequest) -> Result<(), ClientRunError> {
    (ClientRun { process: programs, files: disk, env: &env() }).launch(request).map(|_| ())
}

#[test]
fn every_launch_of_a_client_with_a_run_home_points_home_and_xdg_at_it_and_the_others_keep_theirs() {
    assert_eq!(run_home_dir(&request(RunClient::Opencode, "m", None, RunMode::ReadOnly).paths), "/run/client-home");
    let cases = [
        (RunClient::Opencode, "opencode-go/kimi", None, true),
        (RunClient::Opencode, "litellm-vibe/glm-5.3", None, true),
        (RunClient::Hermes, "glm-5.3", Some("litellm-vibe"), true),
        (RunClient::Hermes, "m", Some("deepinfra"), false),
        (RunClient::Claude, "m", None, false),
        (RunClient::Codex, "m", None, false),
        (RunClient::Agy, "m", None, false),
        (RunClient::Vibe, "m", None, false),
    ];
    for (client, model, provider, has_home) in cases {
        let programs = Programs::answering();
        let disk = owner();
        let _ = run(&disk, &programs, &request(client, model, provider, RunMode::ReadOnly));
        let command = programs.first();
        // The launch's variables: its own in `env`, or as `env NAME=value` words after the wrapper reads a key.
        let words: Vec<String> = command.args.iter().filter(|arg| arg.starts_with("HOME=")).cloned().collect();
        let home_set = command.env.get("HOME").map(|home| format!("HOME={home}")).into_iter().chain(words).collect::<Vec<_>>();
        if has_home {
            assert_eq!(home_set, ["HOME=/run/client-home"], "{client:?} {model}");
            // The home is removed when the launch ends, so a key in it never outlives it.
            assert_eq!(disk.removed.lock().unwrap().as_slice(), ["/run/client-home"], "{client:?}");
        } else {
            assert!(home_set.is_empty(), "{client:?} {model} runs in the home it signed in on");
            assert!(disk.removed.lock().unwrap().is_empty());
        }
    }
}

#[test]
fn every_client_denies_mcp_unless_the_profile_attaches_servers() {
    let launch = |client, model: &str, provider| {
        let programs = Programs::answering();
        let _ = run(&owner(), &programs, &request(client, model, provider, RunMode::ReadOnly));
        programs.first()
    };
    // Pi runs with `--no-mcp` (the pi tests), and a profile's scope refuses an `mcp` entry for it.
    assert!(launch(RunClient::Claude, "m", None).args.contains(&"--strict-mcp-config".to_string()));
    assert!(launch(RunClient::Codex, "m", None).args.contains(&"--ignore-user-config".to_string()));
    assert!(launch(RunClient::Agy, "m", None).args.iter().any(|arg| arg.contains(r#""deny":["mcp(*)"]"#)));
    let hermes = launch(RunClient::Hermes, "m", Some("deepinfra"));
    assert!(hermes.args.contains(&"--safe-mode".to_string()));
    assert_eq!(VIBE_ENV.iter().find(|(name, _)| *name == "VIBE_ENABLE_MCP"), Some(&("VIBE_ENABLE_MCP", "false")));
    assert_eq!(launch(RunClient::Vibe, "m", None).env.get("VIBE_ENABLE_MCP").map(String::as_str), Some("false"));
}

#[test]
fn a_launch_whose_home_cannot_be_made_never_starts_and_a_spawn_failure_still_removes_the_home() {
    let programs = Programs::answering();
    let refused = run(&Disk::default(), &programs, &request(RunClient::Opencode, "litellm-vibe/glm-5.3", None, RunMode::ReadOnly)).expect_err("refused");
    assert!(matches!(&refused, ClientRunError::Gateway(error) if error.code == "unconfined" && error.message.contains("cannot be read")), "{refused:?}");
    assert!(programs.ran.lock().unwrap().is_empty());

    let broken = Programs { launched: Err("spawn failed".into()), ..Programs::answering() };
    let disk = owner();
    let error = run(&disk, &broken, &request(RunClient::Opencode, "opencode-go/kimi", None, RunMode::ReadOnly)).expect_err("spawn");
    assert_eq!(error, ClientRunError::Spawn("spawn failed".into()));
    assert_eq!(disk.removed.lock().unwrap().as_slice(), ["/run/client-home"]);
}

#[test]
fn a_caller_may_not_name_a_variable_the_run_home_sets() {
    use crate::client_run::require_supported_extras;
    for (client, variable) in [(RunClient::Opencode, "XDG_CONFIG_HOME"), (RunClient::Opencode, "OPENCODE_CONFIG"), (RunClient::Hermes, "HERMES_HOME"), (RunClient::Hermes, "HOME")] {
        let extras = RunExtras { env: Some(BTreeMap::from([(variable.to_string(), "/elsewhere".to_string())])), ..RunExtras::default() };
        let error = require_supported_extras(client, &extras).expect_err("refused");
        assert!(error.message.contains(&format!("sets {variable} itself")), "{}", error.message);
    }
}

#[test]
fn a_keyed_launch_with_no_run_home_reads_its_key_and_adds_no_variable() {
    use crate::launchers::{launcher, LaunchExtras};
    let paths = LaunchPaths { export_dir: "/e".into(), prompt_path: "/p".into(), run_dir: None, work_dir: "/w".into(), git_writable: None };
    let input = LaunchRequest { model: "litellm-vibe/m".into(), time_limit_s: 30.0, max_price_usd: None, mode: None, provider: Some("litellm-vibe".into()) };
    for client in [RunClient::Opencode, RunClient::Hermes] {
        let command = (launcher(client).command)("bin", &input, &paths, "ask", Some(&LaunchExtras::default()));
        assert_eq!(command.command, "/bin/sh");
        assert_eq!(command.args[4], "LITELLM_VIBE_KEY");
        assert_eq!(command.args[5], "bin", "no `env` words when there is no home");
    }
}

#[test]
fn the_servers_every_home_declares_are_named_for_the_escape_probe() {
    use crate::home_mcp::declared_servers;
    let env = HashMap::from([("HOME".to_string(), "/h".to_string())]);
    let disk = Disk::default()
        .with("/h/.config/opencode/opencode.json", r#"{"mcp":{"servers":{"memory-api":{},"outline":{}}}}"#)
        .with("/h/.hermes/config.yaml", "model:\n  default: x\nmcp_servers:\n  olcli:\n    command: a\n  drawio:\n    command: b\nproviders:\n  p:\n    x: 1\n")
        .with("/h/.codex/config.toml", "[mcp_servers.idea]\ncommand = \"x\"\n[mcp_servers.n8n]\ncommand = \"y\"\n")
        .with("/h/.vibe/config.toml", "[[mcp_servers]]\nname = \"vibe-one\"\ntransport = \"stdio\"\n");
    assert_eq!(declared_servers(&disk, &env), ["drawio", "idea", "memory-api", "n8n", "olcli", "outline", "vibe-one"]);
    // An OpenCode config that lists servers by name at its top (the v1 shape), the variables that move each home, and a home with nothing.
    let moved = HashMap::from([
        ("HOME".to_string(), "/h".to_string()),
        ("XDG_CONFIG_HOME".to_string(), "/xdg".to_string()),
        ("HERMES_HOME".to_string(), "/hh".to_string()),
        ("CODEX_HOME".to_string(), "/cc".to_string()),
    ]);
    let disk = Disk::default()
        .with("/xdg/opencode/opencode.json", r#"{"mcp":{"alpha":{"type":"local"}}}"#)
        .with("/hh/config.yaml", "mcp_servers: {}\n")
        .with("/cc/config.toml", "model = \"x\"\n");
    assert_eq!(declared_servers(&disk, &moved), ["alpha"]);
    assert!(declared_servers(&Disk::default(), &env).is_empty());
    let broken = Disk::default().with("/h/.config/opencode/opencode.json", "{").with("/h/.codex/config.toml", "= =");
    assert!(declared_servers(&broken, &env).is_empty());
    assert!(declared_servers(&Disk::default().with("/h/.config/opencode/opencode.json", r#"{"mcp":1}"#), &env).is_empty());
    assert!(declared_servers(&Disk::default(), &HashMap::new()).is_empty());
}
