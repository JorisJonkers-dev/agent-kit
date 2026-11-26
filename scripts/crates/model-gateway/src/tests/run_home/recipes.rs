//! The recipe of each client (#903, ADR-0016): the grants it is started with, and what cannot be confined.

use super::*;
use crate::client_confine::{no_launcher, Confinement};
use crate::run_home::{prepare_codex, prepare_vibe};
use crate::tests::confined::{grant_values, LAUNCHER};

/// The grants the launcher was given for one launch.
fn grants_of(disk: &Disk, request: &ClientRunRequest) -> Vec<String> {
    let programs = Programs::answering();
    run(disk, &programs, request).expect("launched");
    let grants = programs.grants.lock().unwrap().clone();
    grants.into_iter().next().expect("a confined launch")
}

fn has(grants: &[String], flag: &str, value: &str) -> bool {
    grant_values(grants, flag).iter().any(|granted| granted == value)
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

/// Files that cannot make a directory.
struct Nowhere(Disk);

impl Files for Nowhere {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        self.0.read_text(path)
    }
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_atomic(path, content)
    }
    fn mkdir_all(&self, _path: &str) -> Result<(), FsError> {
        Err(nothing())
    }
    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        self.0.create_exclusive(path)
    }
    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        self.0.remove_file(path)
    }
    fn modified_ms(&self, path: &str) -> Option<i64> {
        self.0.modified_ms(path)
    }
    fn exists(&self, path: &str) -> Result<bool, FsError> {
        self.0.exists(path)
    }
    fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.append(path, content)
    }
    fn stamp(&self, path: &str) -> Option<crate::ports::FileStamp> {
        self.0.stamp(path)
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_private(path, content)
    }
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_text(path, content)
    }
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
        self.0.temp_dir(parent, prefix)
    }
    fn remove(&self, path: &str) -> Result<(), FsError> {
        self.0.remove(path)
    }
}

/// Files that can make nothing a link or a copy: the default of the port.
struct Plain(Disk);

impl Files for Plain {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        self.0.read_text(path)
    }
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_atomic(path, content)
    }
    fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
        self.0.mkdir_all(path)
    }
    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        self.0.create_exclusive(path)
    }
    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        self.0.remove_file(path)
    }
    fn modified_ms(&self, path: &str) -> Option<i64> {
        self.0.modified_ms(path)
    }
    fn exists(&self, path: &str) -> Result<bool, FsError> {
        self.0.exists(path)
    }
    fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.append(path, content)
    }
    fn stamp(&self, path: &str) -> Option<crate::ports::FileStamp> {
        self.0.stamp(path)
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_private(path, content)
    }
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.0.write_text(path, content)
    }
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
        self.0.temp_dir(parent, prefix)
    }
    fn remove(&self, path: &str) -> Result<(), FsError> {
        self.0.remove(path)
    }
}

#[test]
fn the_grants_of_a_launch_read_the_work_directory_or_write_it_and_write_the_run_directory() {
    let input = LaunchRequest { model: "m".into(), time_limit_s: 30.0, max_price_usd: None, mode: Some(RunMode::ReadOnly), provider: None };
    let mut paths = LaunchPaths { export_dir: "/run/export".into(), prompt_path: "/p".into(), run_dir: None, work_dir: "/w".into(), git_writable: Some(vec!["/g1".into(), "/g2".into()]), git_view: GitView::default() };
    let confinement = Confinement { launcher: LAUNCHER.into(), reads: vec!["/r".into()], run_from: vec!["node".into()], writes: vec!["/own".into()], ports: vec![443, 8443], loopback: true, keychain: true, preferences: true, env: vec!["USER".into()] };
    let names = ["B".to_string(), "A".to_string(), "USER".to_string()];
    // A read-only run reads the work directory, never its git directories, and writes the export directory when it has no run directory.
    assert_eq!(
        confinement.args(&input, &paths, "claude", &names),
        strings(&[
            "confine", "--read", "/w", "--run-from", "claude", "--write", "/run/export", "--read", "/p", "--run-from", "node", "--read", "/r", "--write", "/own", "--net-connect", "443", "--net-connect", "8443", "--net-loopback", "--keychain", "--preferences", "--env", "A", "--env",
            "B", "--env", "USER", "--"
        ])
    );
    // A writing run writes the work directory and the git directories it names, and the run directory.
    // A prompt inside the run directory is readable already and is not granted again.
    paths.run_dir = Some("/run/own".into());
    paths.prompt_path = "/run/own/prompt.md".into();
    let writing = LaunchRequest { mode: Some(RunMode::Write), ..input };
    let bare = Confinement { launcher: LAUNCHER.into(), ..Confinement::default() };
    assert_eq!(bare.args(&writing, &paths, "codex", &[]), strings(&["confine", "--write", "/w", "--write", "/g1", "--write", "/g2", "--run-from", "codex", "--write", "/run/own", "--"]));
    // No prompt file, no read of one.
    let unnamed = LaunchPaths { prompt_path: String::new(), ..paths.clone() };
    assert_eq!(bare.args(&writing, &unnamed, "codex", &[]), bare.args(&writing, &paths, "codex", &[]));
    assert_eq!(no_launcher("opencode"), "opencode: agent-launcher is not on PATH, and a delegate only runs confined to its working directory; re-run agent-kit setup, which installs it");
}

#[test]
fn claude_asks_the_keychain_and_reads_nothing_of_the_owners_home() {
    let grants = grants_of(&owner(), &request(RunClient::Claude, "claude-haiku-5-5", None, RunMode::ReadOnly));
    assert!(grants.contains(&"--keychain".to_string()) && !grants.contains(&"--preferences".to_string()));
    assert!(has(&grants, "--net-connect", "443") && has(&grants, "--read", "/run/work"));
    for name in ["USER", "LOGNAME", "CLAUDE_CODE_TMPDIR", "TMPDIR", "CLAUDE_CODE_DISABLE_AUTO_MEMORY"] {
        assert!(has(&grants, "--env", name), "{name}");
    }
    // Only the run's own directory is writable, and nothing is named under the owner's home.
    assert_eq!(grant_values(&grants, "--write"), ["/run"]);
    assert!(!grants.iter().any(|grant| grant.starts_with("/home/me")), "{grants:?}");
}

#[test]
fn codex_runs_on_a_copy_of_the_owners_login_and_writes_nothing_of_the_owners() {
    let grants = grants_of(&owner(), &request(RunClient::Codex, "gpt", None, RunMode::ReadOnly));
    assert!(grants.contains(&"--keychain".to_string()) && grants.contains(&"--preferences".to_string()));
    assert_eq!(grant_values(&grants, "--write"), ["/run"]);
    assert!(has(&grants, "--env", "CODEX_HOME"));
    // `CODEX_HOME` of the owner's environment names where the login is.
    let named = HashMap::from([("CODEX_HOME".to_string(), "/codex".to_string()), ("HOME".to_string(), "/home/me".to_string())]);
    let home = prepare_codex(&Disk::default().with("/codex/auth.json", "{}"), &named, "/run/h").expect("a home");
    assert_eq!((home.login, home.env["CODEX_HOME"].as_str()), (Some(("/codex/auth.json".to_string(), "/run/h/auth.json".to_string())), "/run/h"));
    assert!(home.writes.is_empty());
    let missing = prepare_codex(&Disk::default(), &env(), "/run/h").expect_err("signed out");
    assert_eq!(missing, "codex: /home/me/.codex/auth.json is not there; sign in with codex login");
    // A home that cannot be written refuses the launch.
    let denied = Disk { write_fails: Some("auth.json"), ..owner() };
    assert!(prepare_codex(&denied, &env(), "/run/h").expect_err("no copy").starts_with("codex: cannot make the launch's home /run/h: EACCES"));
}

#[test]
fn antigravity_writes_its_two_directories_and_has_loopback_and_443_only() {
    let grants = grants_of(&owner(), &request(RunClient::Agy, "gemini-3.8-flash-low", None, RunMode::ReadOnly));
    assert!(grants.contains(&"--keychain".to_string()));
    assert_eq!(grant_values(&grants, "--write"), ["/run", "/home/me/.gemini/antigravity-cli", "/home/me/.gemini/config"]);
    // It serves itself on a loopback port chosen at run time, which no port list can name: loopback at any port, and
    // the rest of the network at 443 only, never unrestricted.
    assert_eq!(grant_values(&grants, "--net-connect"), ["443"]);
    assert!(grants.contains(&"--net-loopback".to_string()));
}

#[test]
fn vibe_runs_in_a_copy_of_its_config_and_agents_and_reads_the_skills_directory_it_walks() {
    let disk = owner().with("/home/me/.vibe/config.toml", "x").with("/home/me/.vibe/agents", "").with("/home/me/.agents/skills", "");
    let grants = grants_of(&disk, &request(RunClient::Vibe, "m", None, RunMode::ReadOnly));
    assert!(grants.contains(&"--keychain".to_string()) && has(&grants, "--net-connect", "443"));
    assert!(has(&grants, "--read", "/home/me/.agents/skills") && has(&grants, "--env", "VIBE_HOME"));
    for copied in ["/run/client-home/vibe/config.toml", "/run/client-home/vibe/agents"] {
        assert!(disk.files.lock().unwrap().contains_key(copied), "{copied}");
    }
    // An owner with no config, agents or skills directory gets a home with none of them, and no extra read.
    let bare = grants_of(&owner(), &request(RunClient::Vibe, "m", None, RunMode::ReadOnly));
    assert_eq!(grant_values(&bare, "--read"), ["/run/work"]);
    let error = prepare_vibe(&Plain(owner().with("/home/me/.vibe/config.toml", "x")), &env(), "/run/h").expect_err("cannot copy");
    assert!(error.starts_with("vibe: cannot copy /home/me/.vibe/config.toml into the launch's home /run/h: "), "{error}");
}

#[test]
fn opencode_on_its_own_providers_writes_the_owners_data_directory_and_on_a_route_nothing_of_the_owners() {
    let signed_in = owner().with("/home/me/.local/share/opencode", "");
    let own = grants_of(&signed_in, &request(RunClient::Opencode, "opencode/free", None, RunMode::ReadOnly));
    assert_eq!(grant_values(&own, "--write"), ["/run", "/home/me/.local/share/opencode"]);
    assert!(has(&own, "--env", "OPENCODE_DB") && grant_values(&own, "--net-connect") == ["443"] && own.contains(&"--net-loopback".to_string()));
    let routed = grants_of(&owner(), &request(RunClient::Opencode, "litellm-vibe/glm-5.3", None, RunMode::ReadOnly));
    assert_eq!(grant_values(&routed, "--write"), ["/run"]);
    assert!(has(&routed, "--env", "LITELLM_VIBE_KEY") && has(&routed, "--env", "XDG_CONFIG_HOME"));
}

const HERMES_LOCAL: &str = "providers:\n  litellm-local:\n    name: litellm-local\n    base_url: http://localhost:4000/v1\n    key_env: LITELLM_LOCAL_KEY\n";

#[test]
fn hermes_reaches_its_providers_port_and_a_provider_of_the_owners_reads_one_file() {
    let routed = grants_of(&owner(), &request(RunClient::Hermes, "glm-5.3", Some("litellm-vibe"), RunMode::ReadOnly));
    assert!(has(&routed, "--net-connect", "443") && has(&routed, "--env", "HERMES_HOME"));
    // A base URL names its own port, and one without a URL has none to name.
    let home = prepare_hermes(&Disk::default().with("/home/me/.hermes/config.yaml", HERMES_LOCAL), &env(), "litellm-local", "/run/h").expect("a home");
    assert_eq!(home.ports, [4000]);
    let unnamed = Disk::default().with("/home/me/.hermes/config.yaml", "providers:\n  litellm-local:\n    name: x\n");
    assert_eq!(prepare_hermes(&unnamed, &env(), "litellm-local", "/run/h").expect("a home").ports, [443]);
    // A provider whose key is in the owner's `.env`: the run's home links that one file, which the launcher reads.
    let keyed = owner().with("/home/me/.hermes/.env", "KEY=x");
    let other = grants_of(&keyed, &request(RunClient::Hermes, "m", Some("deepinfra"), RunMode::ReadOnly));
    assert!(has(&other, "--net-connect", "443") && !other.iter().any(|grant| grant.contains(".hermes/.env")));
    assert_eq!(keyed.text("/run/client-home/.hermes/.env").as_deref(), Some("KEY=x"));
    assert_eq!(keyed.text("/run/client-home/.hermes/config.yaml").as_deref(), Some("mcp_servers: {}\n"));
    // Without a `.env` there is nothing to copy.
    let bare = grants_of(&owner(), &request(RunClient::Hermes, "m", Some("deepinfra"), RunMode::ReadOnly));
    assert_eq!(grant_values(&bare, "--read"), ["/run/work"]);
    let denied = Disk { write_fails: Some("config.yaml"), ..owner().with("/home/me/.hermes/.env", "KEY=x") };
    assert!(prepare_hermes(&denied, &env(), "deepinfra", "/run/h").expect_err("denied").contains("cannot write the launch's home /run/h: EACCES"));
    let denied = Disk { write_fails: Some(".env"), ..owner().with("/home/me/.hermes/.env", "KEY=x") };
    assert!(prepare_hermes(&denied, &env(), "deepinfra", "/run/h").expect_err("no copy").contains("EACCES"));
}

#[test]
fn a_client_that_cannot_be_confined_is_refused_and_leaves_no_home_behind() {
    let programs = Programs::answering();
    // No launcher: no run, and the home made for it goes at once (it may hold a link to the owner's login).
    let no_launcher = HashMap::from([("HOME".to_string(), "/home/me".to_string())]);
    let disk = owner();
    let refused = (ClientRun { process: &programs, files: &disk, env: &no_launcher }).launch(&request(RunClient::Codex, "gpt", None, RunMode::ReadOnly)).expect_err("refused");
    assert!(matches!(&refused, ClientRunError::Gateway(error) if error.code == "unconfined" && error.message.starts_with("codex: agent-launcher is not on PATH")), "{refused:?}");
    assert!(programs.ran.lock().unwrap().is_empty());
    assert_eq!(disk.removed.lock().unwrap().as_slice(), ["/run/client-home"]);
    // A launcher found on `PATH`, and a login that is not there.
    let on_path = HashMap::from([("HOME".to_string(), "/home/me".to_string()), ("PATH".to_string(), "/usr/local/bin".to_string())]);
    let found = owner().with("/usr/local/bin/agent-launcher", "");
    (ClientRun { process: &Programs::answering(), files: &found, env: &on_path }).launch(&request(RunClient::Claude, "m", None, RunMode::ReadOnly)).expect("launched");
    let signed_out = (ClientRun { process: &programs, files: &Disk::default(), env: &env() }).launch(&request(RunClient::Codex, "gpt", None, RunMode::ReadOnly)).expect_err("signed out");
    assert!(matches!(&signed_out, ClientRunError::Gateway(error) if error.message.contains("sign in with codex login")), "{signed_out:?}");
    // A home whose directory cannot be made.
    let broken = Programs::answering();
    let unmakeable = (ClientRun { process: &broken, files: &Nowhere(owner()), env: &env() }).launch(&request(RunClient::Claude, "m", None, RunMode::ReadOnly)).expect_err("no directory");
    assert!(matches!(&unmakeable, ClientRunError::Gateway(error) if error.message.starts_with("claude: cannot make the launch's home /run/client-home: EACCES")), "{unmakeable:?}");
}

#[test]
fn a_launch_built_without_a_confinement_is_the_clients_own_command() {
    use crate::extras::SandboxProfile;
    use crate::launchers::{launcher, LaunchExtras};
    let paths = LaunchPaths { export_dir: "/e".into(), prompt_path: "/p".into(), run_dir: None, work_dir: "/w".into(), git_writable: Some(vec!["/g".into()]), git_view: GitView::default() };
    let input = |mode| LaunchRequest { model: "m".into(), time_limit_s: 30.0, max_price_usd: None, mode: Some(mode), provider: None };
    let build = |client, mode, extras: Option<&LaunchExtras>| (launcher(client).command)("bin", &input(mode), &paths, "ask", extras);
    // Vibe unsets the variables a Study or private session would carry, since the launcher is not there to clear them.
    let vibe = build(RunClient::Vibe, RunMode::ReadOnly, None);
    assert_eq!((vibe.command.as_str(), vibe.args[..5].to_vec()), ("env", strings(&["-u", "MISTRAL_API_KEY", "-u", "VIBE_HOME", "bin"])));
    // Antigravity's own sandbox stays when its own sandbox is the one asked for, and Codex's own sandboxes are its own.
    let sandboxed = LaunchExtras { sandbox: Some(SandboxProfile::ReadOnly), ..LaunchExtras::default() };
    assert!(build(RunClient::Agy, RunMode::ReadOnly, Some(&sandboxed)).args.contains(&"--sandbox".to_string()));
    let read = build(RunClient::Codex, RunMode::ReadOnly, None);
    assert!(read.args.windows(2).any(|pair| pair == ["-s", "read-only"]));
    let write = build(RunClient::Codex, RunMode::Write, None);
    assert!(write.args.windows(2).any(|pair| pair == ["-s", "workspace-write"]) && write.args.windows(2).any(|pair| pair == ["--add-dir", "/g"]));
    assert_eq!(write.command, "bin");
}

#[test]
fn the_servers_and_the_schema_a_caller_attaches_are_run_and_read_by_the_launch() {
    use crate::extras::AttachedServer;
    let disk = owner().with("/assets/finding.schema.json", "{}").with("/p/server.mjs", "").with("/p/packet", "").with("/home/me/.vibe/config.toml", "x");
    let mut vibe = request(RunClient::Vibe, "m", None, RunMode::ReadOnly);
    vibe.extras.mcp_servers = Some(vec![AttachedServer { name: "packet".into(), command: "node".into(), args: strings(&["/p/server.mjs", "--root", "/p/packet", "/p/missing", "relative.mjs"]) }]);
    let grants = grants_of(&disk, &vibe);
    assert_eq!(grant_values(&grants, "--run-from"), ["client", "node"]);
    // Only the paths that are there and are absolute are read.
    assert_eq!(grant_values(&grants, "--read"), ["/run/work", "/p/server.mjs", "/p/packet"]);
    let mut agy = request(RunClient::Agy, "m", None, RunMode::ReadOnly);
    agy.extras.sandbox = Some(crate::extras::SandboxProfile::ReadOnly);
    agy.extras.schema = Some("/assets/finding.schema.json".into());
    assert!(has(&grants_of(&disk, &agy), "--read", "/assets/finding.schema.json"));
    agy.extras.schema = Some("/assets/missing.schema.json".into());
    assert!(!has(&grants_of(&disk, &agy), "--read", "/assets/missing.schema.json"));
}

#[test]
fn what_a_launch_refreshed_in_its_login_goes_back_as_token_fields_only() {
    use crate::run_home::sync_login;
    let owner_text = r#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"old"},"last_refresh":"1","note":"mine"}"#;
    let copy_text = r#"{"auth_mode":"planted","tokens":{"refresh_token":"new"},"last_refresh":"2","extra":"x"}"#;
    let disk = Disk::default().with("/o/auth.json", owner_text).with("/c/auth.json", copy_text);
    sync_login(&disk, "/o/auth.json", "/c/auth.json").expect("synced");
    let kept = json::parse(&disk.text("/o/auth.json").unwrap()).unwrap();
    let kept = kept.as_object().unwrap();
    // The tokens and their time came back; every other member is the owner's own.
    assert_eq!(json::compact(kept.get("tokens").unwrap()), r#"{"refresh_token":"new"}"#);
    assert_eq!((kept.get("auth_mode").and_then(Value::as_str), kept.get("note").and_then(Value::as_str), kept.get("extra")), (Some("chatgpt"), Some("mine"), None));
    // Nothing changed, a missing copy or one that does not parse: the owner's file is left alone.
    let same = Disk::default().with("/o/auth.json", owner_text).with("/c/auth.json", owner_text);
    sync_login(&same, "/o/auth.json", "/c/auth.json").expect("unchanged");
    assert_eq!(same.text("/o/auth.json").as_deref(), Some(owner_text));
    for copy in [None, Some("not json")] {
        let broken = copy.map_or_else(|| Disk::default().with("/o/auth.json", owner_text), |text| Disk::default().with("/o/auth.json", owner_text).with("/c/auth.json", text));
        sync_login(&broken, "/o/auth.json", "/c/auth.json").expect("left alone");
        assert_eq!(broken.text("/o/auth.json").as_deref(), Some(owner_text));
    }
    // A write that fails is said.
    let denied = Disk { write_fails: Some("/o/"), ..Disk::default().with("/o/auth.json", owner_text).with("/c/auth.json", copy_text) };
    assert!(sync_login(&denied, "/o/auth.json", "/c/auth.json").expect_err("denied").starts_with("codex: the refreshed login could not be written back"));
}

#[test]
fn a_refreshed_login_is_put_back_after_the_launch_and_a_failure_is_a_warning() {
    let disk = owner().with("/home/me/.codex/auth.json", r#"{"tokens":{"a":1}}"#);
    let programs = Programs::answering();
    // The fake process refreshes nothing, so the owner's file is unchanged and no warning is made.
    let outcome = (ClientRun { process: &programs, files: &disk, env: &env() }).launch(&request(RunClient::Codex, "gpt", None, RunMode::ReadOnly)).expect("launched");
    assert!(outcome.warnings.is_empty());
    assert_eq!(disk.text("/home/me/.codex/auth.json").as_deref(), Some(r#"{"tokens":{"a":1}}"#));
}

#[test]
fn a_client_that_can_sign_in_from_a_variable_gets_the_credential_and_no_keychain() {
    let run_with = |client, keychain: Vec<(&'static str, &'static str)>, user: bool| {
        let programs = Programs { keychain, ..Programs::answering() };
        let mut environment = env();
        if user {
            environment.insert("USER".to_string(), "owner".to_string());
        }
        let disk = owner();
        (ClientRun { process: &programs, files: &disk, env: &environment }).launch(&request(client, "m", None, RunMode::ReadOnly)).expect("launched");
        let command = programs.first();
        let grants = programs.grants.lock().unwrap()[0].clone();
        (command.env, grants)
    };
    // Claude Code: the access token of the subscription login, from the keychain's JSON.
    let login = r#"{"claudeAiOauth":{"accessToken":"tok-1","refreshToken":"r"}}"#;
    let (env_of, grants) = run_with(RunClient::Claude, vec![("Claude Code-credentials", login)], true);
    assert_eq!(env_of.get("CLAUDE_CODE_OAUTH_TOKEN").map(String::as_str), Some("tok-1"));
    assert!(!grants.contains(&"--keychain".to_string()) && has(&grants, "--env", "CLAUDE_CODE_OAUTH_TOKEN"));
    // Vibe: its key.
    let (env_of, grants) = run_with(RunClient::Vibe, vec![("ai.mistral.vibe", "key-1")], false);
    assert_eq!(env_of.get("MISTRAL_API_KEY").map(String::as_str), Some("key-1"));
    assert!(!grants.contains(&"--keychain".to_string()));
    // No item, an item that is not the login, no user name or a client with no such variable: the keychain is granted
    // where the client has no other way (Antigravity, Codex) or the credential could not be read.
    for (client, keychain, user) in [(RunClient::Claude, vec![], true), (RunClient::Claude, vec![("Claude Code-credentials", "not json")], true), (RunClient::Claude, vec![("Claude Code-credentials", "{}")], true), (RunClient::Claude, vec![], false), (RunClient::Vibe, vec![], false), (RunClient::Agy, vec![], true), (RunClient::Codex, vec![], true)] {
        let (env_of, grants) = run_with(client, keychain, user);
        assert!(grants.contains(&"--keychain".to_string()), "{client:?}");
        assert!(!env_of.contains_key("CLAUDE_CODE_OAUTH_TOKEN") && !env_of.contains_key("MISTRAL_API_KEY"), "{client:?}");
    }
}

#[test]
fn a_home_beside_the_export_directory_is_writable_because_no_run_directory_holds_it() {
    let mut bare = request(RunClient::Claude, "claude-haiku-5-5", None, RunMode::ReadOnly);
    bare.paths.run_dir = None;
    let grants = grants_of(&owner(), &bare);
    // The export directory is writable, and the home is made beside it so that removing it leaves nothing behind.
    assert_eq!(grant_values(&grants, "--write"), ["/run/export", "/run/export.client-home"]);
}
