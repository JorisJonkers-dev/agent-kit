//! The Claude Code, Codex and Hermes launches of the sbx backend: what each is allowed to reach, the one credential
//! each passes by name, and what a refusal says.

use super::sandbox::{answer, gateway_code, healthy, pi_request, sbx_env, Disk, Fixed, Notes, Sbx};
use crate::client_run::ClientRun;
use crate::ports::{Files, ProcessCommand};
use crate::run::{RunClient, RunMode};
use crate::sbx_clients;
use crate::sbx_launch;
use std::sync::Arc;

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const HOME: &str = "/run/export.client-home";

fn image(repo: &str) -> String {
    format!("docker.io/sbx/{repo}@{DIGEST}")
}

fn env_file(disk: &Disk) -> String {
    disk.files.lock().expect("files").get(&format!("{HOME}/sbx.env")).cloned().expect("the env file was written")
}

fn call_with<'a>(calls: &'a [String], start: &str) -> &'a str {
    calls.iter().find(|call| call.starts_with(start)).unwrap_or_else(|| panic!("no {start} call in {calls:?}"))
}

/// `security` answers with Claude Code's keychain item; every other `sbx` call succeeds.
fn with_login(args: &[&str]) -> Result<crate::ports::ProcessResult, String> {
    if args.first() == Some(&"find-generic-password") {
        return answer(r#"{"claudeAiOauth":{"accessToken":"oauth-secret","refreshToken":"refresh-secret"}}"#, "", 0);
    }
    healthy(args)
}

fn claude_env() -> std::collections::HashMap<String, String> {
    let mut env = sbx_env();
    env.insert("USER".to_string(), "me".to_string());
    env
}

#[test]
fn a_claude_launch_passes_its_one_token_by_name_reaches_only_the_anthropic_api_and_never_the_proxy() {
    let disk = Disk::default();
    let sbx = Sbx::new(with_login);
    let env = claude_env();
    let outcome = ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Claude, Some(RunMode::ReadOnly))).expect("a run");
    assert_eq!(outcome.result.stdout, "ok\n");
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    let calls = sbx.calls();
    let create = call_with(&calls, "sbx create ");
    assert!(create.contains(&format!("-t {} shell ", image("claude-image"))) && create.contains(&format!(" {HOME} ")), "{create}");
    assert!(create.contains(" /run/work:ro"), "{create}");
    assert!(call_with(&calls, "sbx policy").ends_with(" api.anthropic.com:443"), "{calls:?}");
    let exec = call_with(&calls, "sbx exec ");
    assert!(exec.contains(&format!("--env-file {HOME}/sbx.env")) && exec.contains(" claude -p --model glm-5.3 "), "{exec}");
    assert!(exec.contains("--permission-mode plan --allowedTools Read,Grep,Glob"), "{exec}");
    // The token is in the env file under its name; no argument holds it, nor does the keychain item's other field.
    assert!(!calls.iter().any(|call| call.contains("oauth-secret") || call.contains("refresh-secret")), "{calls:?}");
    let file = env_file(&disk);
    assert!(file.lines().any(|line| line == "CLAUDE_CODE_OAUTH_TOKEN=oauth-secret"), "{file}");
    assert!(!file.contains("refresh-secret") && !file.contains("LITELLM"), "{file}");
    assert!(disk.removed.lock().expect("removed").contains(&format!("{HOME}/sbx.env")));
    assert!(calls.last().expect("calls").starts_with("sbx rm --force mg-"));
}

#[test]
fn a_claude_launch_with_no_token_to_hand_in_is_refused_for_its_keychain_and_makes_nothing() {
    let disk = Disk::default();
    let sbx = Sbx::new(|args| if args.first() == Some(&"find-generic-password") { answer("", "not found", 44) } else { healthy(args) });
    let env = claude_env();
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Claude, Some(RunMode::ReadOnly))).expect_err("refused"));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.starts_with("claude: the sbx plan holds no way to give keychain:"), "{message}");
    assert!(!sbx.calls().iter().any(|call| call.starts_with("sbx create")), "{:?}", sbx.calls());
    assert!(disk.removed.lock().expect("removed").contains(&HOME.to_string()), "the run home goes");
}

const AUTH: &str = r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"id","access_token":"old-access","refresh_token":"old-refresh"},"last_refresh":"2026-11-01T00:00:00Z"}"#;
const REFRESHED: &str = r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":"planted","tokens":{"id_token":"id","access_token":"new-access","refresh_token":"new-refresh"},"last_refresh":"2026-11-27T11:59:00Z"}"#;

#[test]
fn a_codex_launch_writes_its_own_copy_of_the_login_reaches_openai_and_hands_the_refreshed_tokens_back() {
    let disk = Arc::new(Disk::default());
    disk.files.lock().expect("files").insert("/home/me/.codex/auth.json".to_string(), AUTH.to_string());
    // The client refreshes the tokens in the copy it was given, which is where the run home is mounted written.
    let inside = Arc::clone(&disk);
    let sbx = Sbx::new(healthy).running(move |_| {
        inside.files.lock().expect("files").insert(format!("{HOME}/auth.json"), REFRESHED.to_string());
        answer("ok\n", "", 0)
    });
    let env = sbx_env();
    let outcome = ClientRun { process: &sbx, files: &*disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Codex, Some(RunMode::ReadOnly))).expect("a run");
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    let calls = sbx.calls();
    let create = call_with(&calls, "sbx create ");
    assert!(create.contains(&format!("-t {} shell ", image("codex-image"))) && create.contains(&format!(" {HOME}")), "{create}");
    assert!(!create.contains("/home/me/.codex"), "the owner's login is never mounted: {create}");
    assert!(call_with(&calls, "sbx policy").ends_with(" chatgpt.com:443,api.openai.com:443,auth.openai.com:443"), "{calls:?}");
    // The microVM is the sandbox: Codex's own is off, as for the confined recipe (ADR-0019).
    let exec = call_with(&calls, "sbx exec ");
    assert!(exec.contains(" codex exec --json ") && exec.contains(" -s danger-full-access "), "{exec}");
    let file = env_file(&disk);
    assert!(file.lines().any(|line| line == format!("CODEX_HOME={HOME}")), "{file}");
    assert!(!file.contains("access"), "no token is in the env file: {file}");
    // Only the tokens and their time go back to the owner's file; the rest of the copy never does.
    let owner = disk.files.lock().expect("files").get("/home/me/.codex/auth.json").cloned().expect("the owner's login");
    let owner = crate::json::parse(&owner).expect("JSON");
    let owner = owner.as_object().expect("an object");
    assert_eq!(owner.get("OPENAI_API_KEY"), Some(&crate::json::Value::Null));
    assert_eq!(owner.get("last_refresh").and_then(|value| value.as_str()), Some("2026-11-27T11:59:00Z"));
    let tokens = owner.get("tokens").and_then(|value| value.as_object()).expect("tokens");
    assert_eq!(tokens.get("access_token").and_then(|value| value.as_str()), Some("new-access"));
    assert_eq!(tokens.get("refresh_token").and_then(|value| value.as_str()), Some("new-refresh"));
}

#[test]
fn a_codex_launch_without_a_login_is_refused_before_any_sandbox() {
    let disk = Disk::default();
    let sbx = Sbx::new(healthy);
    let env = sbx_env();
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Codex, Some(RunMode::ReadOnly))).expect_err("refused"));
    assert_eq!(code, "unconfined");
    assert!(message.contains("sign in with codex login"), "{message}");
    assert!(!sbx.calls().iter().any(|call| call.starts_with("sbx create")));
}

const HERMES_CONFIG: &str = "providers:\n  litellm-vibe:\n    base_url: http://localhost:4000/v1\n    key_env: LITELLM_VIBE_KEY\n    api_mode: chat_completions\n";

#[test]
fn a_hermes_launch_on_a_litellm_route_passes_the_key_by_name_and_reaches_the_proxy_by_both_loopback_names() {
    let disk = Disk::default();
    disk.files.lock().expect("files").insert("/home/me/.hermes/config.yaml".to_string(), HERMES_CONFIG.to_string());
    let sbx = Sbx::new(|args| if args.first() == Some(&"get") { answer("proxy-secret\n", "", 0) } else { healthy(args) });
    let env = sbx_env();
    let mut request = pi_request(RunClient::Hermes, Some(RunMode::ReadOnly));
    request.binary = "hermes".to_string();
    let outcome = ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&request).expect("a run");
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    let calls = sbx.calls();
    let create = call_with(&calls, "sbx create ");
    assert!(create.contains(&format!("-t {} shell", image("hermes-agent-image"))), "{create}");
    assert!(call_with(&calls, "sbx policy").ends_with(" host.docker.internal:4000,localhost:4000"), "{calls:?}");
    let exec = call_with(&calls, "sbx exec ");
    assert!(exec.contains(" sh -c ") && exec.contains("exec hermes \"$@\" hermes -z ") && exec.contains("--provider litellm-vibe"), "{exec}");
    assert!(!calls.iter().any(|call| call.starts_with("sbx") && call.contains("proxy-secret")), "{calls:?}");
    let file = env_file(&disk);
    assert!(file.lines().any(|line| line.ends_with("=proxy-secret") && line.starts_with("LITELLM_")), "{file}");
    let written = disk.files.lock().expect("files").get(&format!("{HOME}/.hermes/config.yaml")).cloned().expect("the run home's config");
    assert!(written.contains("base_url: http://host.docker.internal:4000/v1"), "{written}");
    assert!(written.ends_with("model:\n  provider: litellm-vibe\n  default: glm-5.3\n"), "{written}");
}

#[test]
fn a_hermes_launch_on_a_provider_of_the_owners_home_is_refused_because_no_route_names_a_proxy() {
    let disk = Disk::default();
    let sbx = Sbx::new(healthy);
    let env = sbx_env();
    let mut request = pi_request(RunClient::Hermes, Some(RunMode::ReadOnly));
    request.launch.provider = Some("openrouter".to_string());
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&request).expect_err("refused"));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.contains("hermes: the run home's config.yaml names no base_url"), "{message}");
    assert!(!sbx.calls().iter().any(|call| call.starts_with("sbx create")));
}

#[test]
fn antigravity_is_refused_before_sbx_is_asked_and_says_why() {
    let disk = Disk::default();
    let sbx = Sbx::new(healthy);
    let env = sbx_env();
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&pi_request(RunClient::Agy, Some(RunMode::ReadOnly))).expect_err("refused"));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.starts_with("agy: the sbx backend does not run this client: its sign-in is the macOS login keychain"), "{message}");
    assert!(sbx.calls().is_empty());
}

#[test]
fn the_fixed_hosts_belong_to_claude_and_codex_alone_and_the_hermes_route_is_read_from_its_yaml() {
    assert_eq!(sbx_clients::fixed_hosts(RunClient::Claude), Some(vec!["api.anthropic.com:443".to_string()]));
    assert_eq!(sbx_clients::fixed_hosts(RunClient::Codex).map(|hosts| hosts.len()), Some(3));
    for client in [RunClient::Pi, RunClient::Opencode, RunClient::Vibe, RunClient::Hermes, RunClient::Agy] {
        assert_eq!(sbx_clients::fixed_hosts(client), None);
    }
    assert_eq!(sbx_launch::program_of(RunClient::Claude), Some("claude"));
    assert_eq!(sbx_launch::program_of(RunClient::Codex), Some("codex"));
    assert_eq!(sbx_launch::program_of(RunClient::Hermes), Some("hermes"));
    assert_eq!(sbx_launch::config_path(RunClient::Hermes, "/run/home"), "/run/home/.hermes/config.yaml");

    assert_eq!(sbx_clients::hermes_allowed("providers:\n  p:\n    base_url: \"https://litellm.example.com/v1\"\n").expect("rules"), ["litellm.example.com:443"]);
    assert_eq!(sbx_clients::hermes_allowed("providers:\n  p:\n    base_url: 'http://127.0.0.1:4000'\n").expect("rules"), ["host.docker.internal:4000", "localhost:4000"]);
    for config in ["mcp_servers: {}\n", "providers:\n  p:\n    base_url:\n", ""] {
        assert!(sbx_clients::hermes_allowed(config).expect_err("refused").message.contains("names no base_url"), "{config}");
    }
    assert!(sbx_clients::hermes_allowed("base_url: ftp://x:21\n").expect_err("refused").message.contains("names no host"));

    assert_eq!(sbx_clients::hermes_rewritten("a:\n  base_url: http://localhost:4000/v1\n").as_deref(), Some("a:\n  base_url: http://host.docker.internal:4000/v1\n"));
    assert_eq!(sbx_clients::hermes_rewritten("base_url: http://127.0.0.1/v1").as_deref(), Some("base_url: http://host.docker.internal/v1"));
    assert_eq!(sbx_clients::hermes_rewritten("base_url: https://litellm.example.com/v1\n"), None);
    assert_eq!(sbx_clients::hermes_rewritten("base_url: nohost\n"), None);
    assert_eq!(sbx_clients::hermes_rewritten("mcp_servers: {}\n"), None);
}

#[test]
fn a_wrapped_hermes_command_is_unwrapped_like_opencodes_and_its_key_is_read_outside() {
    let words = ["-c", crate::launchers::HERMES_KEYED, "hermes-keyed", "agent-kit-secret", "LITELLM_VIBE_KEY", "env", "HERMES_HOME=/run/home/.hermes", "/opt/agent-launcher", "confine", "--write", "/run/home", "--env", "LITELLM_VIBE_KEY", "--env", "HERMES_HOME", "--", "hermes", "-z", "say ok"];
    let command = ProcessCommand { cwd: Some("/run/work".to_string()), ..ProcessCommand::new("/bin/sh", &words) };
    let sbx = sbx_launch::translate(RunClient::Hermes, "mg-test", &command, "base_url: https://litellm.example.com/v1\n", &|key| Ok(format!("value-of-{key}"))).expect("a translation");
    assert_eq!(sbx.program, "hermes");
    assert_eq!(sbx.args, ["-z", "say ok"]);
    assert_eq!(sbx.env, ["LITELLM_VIBE_KEY=value-of-LITELLM_VIBE_KEY", "HERMES_HOME=/run/home/.hermes"]);
    assert_eq!(sbx.allow, ["litellm.example.com:443"]);
    let _ = Notes::default();
    let _: &dyn Files = &Disk::default();
}

#[test]
fn only_hermes_is_started_through_the_shell_that_links_its_install_state() {
    let args = vec!["-z".to_string(), "say ok".to_string()];
    assert_eq!(sbx_clients::words_of("codex", &args), ["codex", "-z", "say ok"]);
    let words = sbx_clients::words_of("hermes", &args);
    assert_eq!(&words[..2], ["sh", "-c"]);
    assert!(words[2].contains("ln -s \"/home/agent/.hermes/$d\" \"$HERMES_HOME/$d\"") && words[2].ends_with("exec hermes \"$@\""), "{}", words[2]);
    assert_eq!(&words[3..], ["hermes", "-z", "say ok"]);
}

#[test]
fn hermes_gets_the_model_block_its_image_reads_unless_it_has_one_or_the_launch_names_none() {
    let args: Vec<String> = ["-z", "hi", "-m", "m1", "--provider", "p1"].iter().map(|word| (*word).to_string()).collect();
    assert_eq!(sbx_clients::with_model(RunClient::Hermes, "a: 1\n", &args), "a: 1\nmodel:\n  provider: p1\n  default: m1\n");
    assert_eq!(sbx_clients::with_model(RunClient::Hermes, "a: 1", &args), "a: 1\nmodel:\n  provider: p1\n  default: m1\n");
    assert_eq!(sbx_clients::with_model(RunClient::Hermes, "model:\n  provider: x\n", &args), "model:\n  provider: x\n");
    assert_eq!(sbx_clients::with_model(RunClient::Hermes, "a: 1\n", &args[..2]), "a: 1\n");
    assert_eq!(sbx_clients::with_model(RunClient::Pi, "a: 1\n", &args), "a: 1\n");
}
