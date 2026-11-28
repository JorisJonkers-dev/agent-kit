//! The Docker Sandboxes (`sbx`) run of one Pi, OpenCode, Vibe, Claude Code, Codex or Hermes launch: the only way the gateway starts a client.
//!
//! The plan command the launch is built as is the source: its grants become one sandbox's mounts
//! (`SbxPlan`), its variables pass by name with their values from the command, and its route's base URL decides the
//! network rules the sandbox gets, widened or narrowed by the run's reach (`network.rs`). A run is `sbx create`
//! (named, from the client's prebuilt image pinned by digest), `sbx policy allow network --sandbox` for the route and
//! the reach, `sbx policy deny network --sandbox` for the global allows the reach does not need, the package proxy's
//! custom secret, `sbx exec` of the client with the watch on its output, and `sbx rm --force` whatever happened
//! before (which also removes the sandbox's rules and secret). Everything goes through the `Process` port, so tests use a fake and no real sbx starts.

use crate::error::GatewayError;
use crate::json::{self, Value};
use crate::launchers::{HERMES_KEYED, OPENCODE_KEYED, SECRET_TOOL};
use crate::network::{self, Egress, NetworkScope, Reach};
use crate::pi_launch::url_port;
use crate::ports::{Files, LaunchWatch, Launched, Process, ProcessCommand, ProcessResult};
use crate::run::RunClient;
use crate::sandbox::{Mount, SbxPlan};
use crate::text::join;
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

/// The program that drives Docker Sandboxes.
pub(crate) const SBX: &str = "sbx";

/// The longest one `sbx` call may take: a cold image pull is about 16 s, a warm create about 2.5 s.
pub(crate) const SBX_TIMEOUT_MS: u64 = 120_000;

/// What the owner runs to get `sbx`, named in every refusal that is about its absence.
pub const SBX_INSTALL: &str = "brew trust docker/tap && brew install docker/tap/sbx, then sign in with sbx login";

/// The host name a sandbox reaches the host's loopback by.
pub const HOST_FROM_SANDBOX: &str = "host.docker.internal";

/// The client's program in its image, which is also the client's name: Pi, OpenCode, Vibe, Claude Code, Codex and Hermes
/// are the clients the sbx backend runs. Every other client (Antigravity) is refused before anything is made.
pub fn program_of(client: RunClient) -> Option<&'static str> {
    match client {
        RunClient::Pi => Some("pi"),
        RunClient::Opencode => Some("opencode"),
        RunClient::Vibe => Some("vibe"),
        RunClient::Claude => Some("claude"),
        RunClient::Codex => Some("codex"),
        RunClient::Hermes => Some("hermes"),
        _ => None,
    }
}

/// A client the backend runs, or the refusal that says it does not: Antigravity's sign-in is the macOS login
/// keychain and its state is in the owner's ~/.gemini, which no sandbox holds, and no gateway worker runs outside one.
pub fn require_client(client: RunClient) -> Result<(), GatewayError> {
    match program_of(client) {
        Some(_) => Ok(()),
        None => Err(GatewayError::new("sandbox-unsupported", format!("{}: no sandbox can run this client, and every gateway worker runs in one: its sign-in is the macOS login keychain and its state is in the owner's ~/.gemini, which no sandbox holds", client.as_str()))),
    }
}

/// What must hold before a launch makes anything: an `sbx` that answers, and no stored sbx service secret the run's
/// profile does not name. A writing run is the same launch as a read-only one: its worktree and the git directories
/// it writes are the written mounts of its grants. Refused with the reason and the command that fixes it.
pub fn preflight(process: &dyn Process, scope: &NetworkScope) -> Result<(), GatewayError> {
    match process.exec(SBX, &["version"], Some(SBX_TIMEOUT_MS)) {
        Ok(result) if result.exit_code == 0 && !result.timed_out => {}
        Ok(result) => return Err(unavailable(&format!("sbx version exited {}", result.exit_code))),
        Err(reason) => return Err(unavailable(&reason)),
    }
    let listing = sbx_stdout(process, &["secret", "ls", "-g", "--json"]).map_err(|reason| GatewayError::new("sandbox-unsafe", format!("the run cannot be checked for stored sbx service secrets ({reason})")))?;
    network::require_no_service_secrets(&listing, &scope.secrets)
}

/// The standard output of one `sbx` call that must succeed.
fn sbx_stdout(process: &dyn Process, args: &[&str]) -> Result<String, String> {
    match process.exec(SBX, args, Some(SBX_TIMEOUT_MS)) {
        Ok(ProcessResult { exit_code: 0, timed_out: false, stdout, .. }) => Ok(stdout),
        Ok(result) => Err(format!("sbx {} exited {}: {}", args.join(" "), result.exit_code, result.stderr.trim())),
        Err(reason) => Err(format!("sbx {} did not run: {reason}", args.join(" "))),
    }
}

fn unavailable(reason: &str) -> GatewayError {
    GatewayError::new("sandbox-unavailable", format!("sbx is not usable ({reason}); install it with {SBX_INSTALL}"))
}

/// A launch's name in sbx: unique per process and per moment, so no run reuses another's sandbox.
pub fn sandbox_name() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_nanos());
    format!("mg-{}-{:x}", std::process::id(), nanos)
}

/// Removes every `mg-*` sandbox whose gateway is gone: a gateway killed before it could remove its run's sandbox
/// leaves it behind, and its name (`mg-<gateway pid>-<time>`) says which gateway made it. A sandbox of this very
/// process or of a gateway still alive is another run's and is kept. Nothing here fails a launch: what cannot be
/// listed or removed is returned as warnings.
pub fn prune_orphans(process: &dyn Process) -> Vec<String> {
    let mut warnings = Vec::new();
    for name in orphans(process) {
        if !matches!(process.exec(SBX, &["rm", "--force", &name], Some(SBX_TIMEOUT_MS)), Ok(result) if result.exit_code == 0) {
            warnings.push(format!("the sandbox {name} of an ended run was not removed; remove it with sbx rm --force {name}"));
        }
    }
    warnings
}

/// The `mg-*` sandboxes whose gateway is gone, in the order sbx lists them. None when sbx cannot list.
pub fn orphans(process: &dyn Process) -> Vec<String> {
    let listed = match process.exec(SBX, &["ls", "-q"], Some(SBX_TIMEOUT_MS)) {
        Ok(result) if result.exit_code == 0 => result.stdout,
        _ => return Vec::new(),
    };
    let own = std::process::id();
    let mut names = Vec::new();
    for name in listed.lines().map(str::trim) {
        let Some(pid) = name.strip_prefix("mg-").and_then(|rest| rest.split('-').next()).and_then(|pid| pid.parse::<u32>().ok()) else {
            continue;
        };
        if pid != own && !process.is_alive(pid) {
            names.push(name.to_string());
        }
    }
    names
}

/// The sandbox side of one launch, translated from its plan command and checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SbxLaunch {
    pub name: String,
    pub program: &'static str,
    /// The image reference the sandbox starts from, pinned by digest (`docker.io/sbx/<repo>@sha256:...`).
    pub image: String,
    /// Where the `NAME=value` lines for `sbx exec --env-file` are written for the run, 0600, and removed after it.
    pub env_file: String,
    pub plan: SbxPlan,
    /// The client's own arguments, after its binary in the confined command.
    pub args: Vec<String>,
    /// `NAME=value` for each variable the launch passes, in the plan's order.
    pub env: Vec<String>,
    /// The rules `sbx policy allow network` adds for the sandbox: the route's hosts first, then the run's reach.
    pub allow: Vec<String>,
    /// The global allows the run's reach does not need, which `sbx policy deny network` denies for the sandbox.
    pub deny: Vec<String>,
    /// The proxy key and the placeholder the sandbox holds for it; none where the machine has no key.
    pub credential: Option<ProxyCredential>,
    /// What the run records of the reach it got.
    pub reach: String,
}

/// The package proxy's credential: the key stays on the host, in the sbx custom secret, and the sandbox only ever
/// holds the placeholder the host proxy swaps for it on requests to the proxy host.
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyCredential {
    pub placeholder: String,
    key: String,
}

impl std::fmt::Debug for ProxyCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ProxyCredential").field("placeholder", &self.placeholder).field("key", &"<hidden>").finish()
    }
}

impl SbxLaunch {
    /// `sbx create`: the named sandbox from the client's prebuilt image, with the plan's mounts. The image is pinned by
    /// digest, and sbx pulls it only when the machine does not hold it yet.
    pub fn create_args(&self) -> Vec<String> {
        let mut args = strings(&["create", "--name", &self.name, "--no-input", "--pull", "missing", "-t", &self.image, "shell"]);
        // sbx takes the first mount as the primary workspace and refuses it read-only, so the written mounts go first.
        let (written, read): (Vec<&Mount>, Vec<&Mount>) = self.plan.mounts.iter().partition(|mount| !mount.read_only);
        args.extend(written.into_iter().chain(read).map(Mount::arg));
        args
    }

    /// `sbx policy allow network --sandbox`, for the route's hosts and the reach's, in the one rule the sandbox gets.
    pub fn policy_args(&self) -> Vec<String> {
        strings(&["policy", "allow", "network", "--sandbox", &self.name, &self.allow.join(",")])
    }

    /// `sbx policy deny network --sandbox`, for the global allows the reach does not need; none when it needs them all.
    pub fn deny_args(&self) -> Option<Vec<String>> {
        (!self.deny.is_empty()).then(|| strings(&["policy", "deny", "network", "--sandbox", &self.name, &self.deny.join(",")]))
    }

    /// `sbx secret set-custom`, scoped to the sandbox and to the package proxy's host; the key is read from stdin.
    pub fn secret_args(&self) -> Option<Vec<String>> {
        self.credential.as_ref().map(|credential| strings(&["secret", "set-custom", "--sandbox", &self.name, "--host", network::PROXY_HOST, "--placeholder", &credential.placeholder, "--value-stdin"]))
    }

    /// `sbx exec`: the client in the sandbox, from the plan's workdir, with the variables the launch names read from
    /// the env file, so no value is ever part of the argument list.
    pub fn exec_args(&self) -> Vec<String> {
        let mut args = strings(&["exec", "-i", "-w", &self.plan.workdir]);
        if !self.env.is_empty() {
            args.extend(strings(&["--env-file", &self.env_file]));
        }
        args.push(self.name.clone());
        args.extend(crate::sbx_clients::words_of(self.program, &self.args));
        args
    }
}

/// What the sbx side of a launch came to: the client's result (or why it did not start), and what could not be cleaned up.
pub struct SbxRun {
    pub ran: Result<Launched, String>,
    pub warnings: Vec<String>,
}

/// Translates a plan command into its sandbox. `inner` is the plan command itself (`sbx-plan grants ... -- client
/// args`), which OpenCode's keyed wrapper hides behind `/bin/sh`: the wrapper's `env` pairs and its key
/// name are unwrapped here, and the key's value comes from `read_key`, which reads the secret store the wrapper would.
pub fn translate(client: RunClient, name: &str, command: &ProcessCommand, route_text: &str, read_key: &dyn Fn(&str) -> Result<String, GatewayError>) -> Result<SbxLaunch, GatewayError> {
    let program = program_of(client).ok_or_else(|| GatewayError::new("sandbox-unsupported", format!("{}: no sbx program", client.as_str())))?;
    let (inner, wrapped, key) = unwrapped(command);
    let plan = SbxPlan::of_command(client.as_str(), &inner).ok_or_else(|| GatewayError::new("sandbox-unsupported", format!("{}: the launch carries no grants, so it has no sandbox plan", client.as_str())))?;
    if plan.mounts.iter().all(|mount| mount.read_only) {
        return Err(GatewayError::new("sandbox-unsupported", format!("{}: the sbx plan has no written mount, and sbx needs one as its primary workspace", client.as_str())));
    }
    // Codex's login is the copy of `auth.json` in its run home, which the sandbox mounts written; the keychain the
    // confined recipe also grants it is not a way it signs in here.
    let gaps: Vec<&String> = plan.gaps.iter().filter(|gap| !gap.starts_with("network:") && !(client == RunClient::Codex && gap.starts_with("keychain:"))).collect();
    if !gaps.is_empty() {
        let names: Vec<&str> = gaps.iter().map(|gap| gap.as_str()).collect();
        return Err(GatewayError::new("sandbox-unsupported", format!("{}: the sbx plan holds no way to give {}", client.as_str(), names.join("; "))));
    }
    // `of_command` has found the `--` that separates the grants from the client, so the client follows it.
    let end = inner.args.iter().position(|word| word == "--").unwrap_or(inner.args.len());
    let args = inner.args.get(end + 2..).unwrap_or_default().to_vec();
    let mut values: BTreeMap<String, String> = inner.env.clone();
    values.extend(wrapped);
    if let Some(key) = key {
        values.insert(key.clone(), read_key(&key)?);
    }
    let env = plan.env.iter().filter_map(|name| values.get(name).map(|value| format!("{name}={value}"))).collect();
    let allow = match client {
        RunClient::Vibe => crate::sbx_vibe::allowed(route_text, &args)?,
        RunClient::Hermes => crate::sbx_clients::hermes_allowed(route_text)?,
        _ => match crate::sbx_clients::zen_hosts(client, &args) {
            Some(hosts) => hosts,
            None => crate::sbx_clients::fixed_hosts(client).map_or_else(|| allowed_hosts(route_text), Ok)?,
        },
    };
    Ok(SbxLaunch { name: name.to_string(), program, image: String::new(), env_file: String::new(), plan, args, env, allow, deny: Vec::new(), credential: None, reach: String::new() })
}

/// Gives the launch its network reach and the package proxy: the rules for the run's reach (read against the
/// machine's global allows), the task's own hosts, the proxy key as an sbx custom secret (the run's own npm config and
/// the managers' settings carry only its placeholder), and the words the run records. A machine with no proxy key
/// goes on without a credential, and the recorded reach says so. `npmrc` is where the run's npm user config is written.
pub fn apply_network(sbx: &mut SbxLaunch, process: &dyn Process, files: &dyn Files, env: &HashMap<String, String>, scope: &NetworkScope, task_hosts: &[String], npmrc: &str) -> Result<(), GatewayError> {
    let extras = task_hosts.iter().map(|entry| network::extra_rule(entry)).collect::<Result<Vec<_>, _>>()?;
    let global = if scope.reach == Reach::OpenWeb {
        Vec::new()
    } else {
        let listing = sbx_stdout(process, &["policy", "ls", "--json"]).map_err(|reason| GatewayError::new("sandbox-unsafe", format!("the machine's global sbx policy could not be read ({reason}), so the run's network reach cannot be narrowed")))?;
        network::global_allows(&listing)?
    };
    let egress: Egress = network::plan(scope.reach, &sbx.allow, &extras, &global)?;
    sbx.credential = network::proxy_key(files, env).map(|key| ProxyCredential { placeholder: network::placeholder(&sbx.name), key });
    let placeholder = sbx.credential.as_ref().map(|credential| credential.placeholder.as_str());
    let credential = match placeholder {
        Some(_) => "key injected by the host proxy, only a placeholder in the sandbox".to_string(),
        None => format!("no key on this machine ({} or {} under the config directory), so the sandbox reaches it anonymously", network::KEY_VARIABLE, network::KEY_FILE),
    };
    files.write_private(npmrc, &network::npmrc_text(placeholder)).map_err(|error| GatewayError::new("sandbox-unsupported", format!("the run's npm config was not written: {}", error.message)))?;
    sbx.env.extend(network::package_env(placeholder, npmrc));
    sbx.reach = network::describe(scope, &egress, &credential);
    sbx.allow = egress.allow;
    sbx.deny = egress.deny;
    Ok(())
}

/// The plan command with OpenCode's keyed wrapper taken off: the key's name, and the `env NAME=value` pairs the
/// wrapper sets before the launch, with the launch itself. A command with no wrapper is returned as it is.
fn unwrapped(command: &ProcessCommand) -> (ProcessCommand, BTreeMap<String, String>, Option<String>) {
    let words = &command.args;
    if command.command != "/bin/sh" || ![OPENCODE_KEYED, HERMES_KEYED].contains(&words.get(1).map_or("", String::as_str)) {
        return (command.clone(), BTreeMap::new(), None);
    }
    let key = words.get(4).cloned();
    let mut at = 5;
    let mut pairs = BTreeMap::new();
    if words.get(at).map(String::as_str) == Some("env") {
        at += 1;
        while let Some((name, value)) = words.get(at).and_then(|word| word.split_once('=')) {
            pairs.insert(name.to_string(), value.to_string());
            at += 1;
        }
    }
    let inner = ProcessCommand {
        command: words.get(at).cloned().unwrap_or_default(),
        args: words.get(at + 1..).unwrap_or_default().to_vec(),
        cwd: command.cwd.clone(),
        env: command.env.clone(),
        timeout_ms: command.timeout_ms,
        stdin: command.stdin.clone(),
    };
    (inner, pairs, key)
}

/// The `host:port` rules a route needs, read from the base URL of the route's own file: a loopback proxy is
/// reached as `host.docker.internal`, and that name and `localhost` are both allowed (measured: the proxy matches
/// the rule against `localhost`). A route with no base URL is refused, since sbx allows only the proxy named there.
fn allowed_hosts(route_text: &str) -> Result<Vec<String>, GatewayError> {
    let url = json::parse(route_text).ok().and_then(|value| base_url_in(&value)).ok_or_else(|| GatewayError::new("sandbox-unsupported", "the route names no base URL; the sbx backend allows only the LiteLLM proxy the route names"))?;
    host_rules(&url)
}

/// The rules for one base URL: the host and port, with a loopback host named both ways.
pub(crate) fn host_rules(url: &str) -> Result<Vec<String>, GatewayError> {
    let (host, port) = split_host(url).ok_or_else(|| GatewayError::new("sandbox-unsupported", "the route's base URL names no host the sbx policy can allow"))?;
    Ok(if host == "localhost" || host == "127.0.0.1" { vec![format!("{HOST_FROM_SANDBOX}:{port}"), format!("localhost:{port}")] } else { vec![format!("{host}:{port}")] })
}

/// The first `baseUrl` (pi) or `baseURL` (OpenCode) string in a config, at any depth.
fn base_url_in(value: &Value) -> Option<String> {
    match value {
        Value::Obj(object) => {
            if let Some(url) = ["baseUrl", "baseURL"].iter().find_map(|key| object.get(key).and_then(Value::as_str)) {
                return Some(url.to_string());
            }
            object.entries().into_iter().find_map(|(_, child)| base_url_in(child))
        }
        Value::Arr(items) => items.iter().find_map(base_url_in),
        _ => None,
    }
}

/// The host and port of an http or https URL.
fn split_host(url: &str) -> Option<(String, u16)> {
    let port = url_port(url)?;
    let rest = url.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    let host = authority.rsplit_once(':').filter(|(_, digits)| digits.bytes().all(|byte| byte.is_ascii_digit())).map_or(authority, |(host, _)| host);
    (!host.is_empty()).then(|| (host.to_string(), port))
}

/// The route's config rewritten so a loopback proxy is reached through `host.docker.internal`, which the sandbox
/// sees; `None` when no rewrite is needed. The text is the config's own, with only the base URL changed.
pub fn rewritten_config(client: RunClient, route_text: &str) -> Option<String> {
    match client {
        RunClient::Vibe => return crate::sbx_vibe::rewritten(route_text),
        RunClient::Hermes => return crate::sbx_clients::hermes_rewritten(route_text),
        _ => {}
    }
    let url = json::parse(route_text).ok().and_then(|value| base_url_in(&value))?;
    let (host, _) = split_host(&url)?;
    if host != "localhost" && host != "127.0.0.1" {
        return None;
    }
    Some(route_text.replacen(&url, &url.replacen(&host, HOST_FROM_SANDBOX, 1), 1))
}

/// The config file a route's base URL is read from: Pi's `models.json` in the launch's agent directory, OpenCode's
/// `opencode.json` in its run home.
pub fn config_path(client: RunClient, dir: &str) -> String {
    match client {
        RunClient::Pi => join(&[dir, "models.json"]),
        RunClient::Vibe => join(&[dir, "vibe", "config.toml"]),
        RunClient::Hermes => join(&[dir, ".hermes", "config.yaml"]),
        _ => join(&[dir, "opencode.json"]),
    }
}

/// Runs one translated launch: create, allow, exec, then remove, whatever came before. The variables go to the env
/// file just before the exec and that file is removed right after it, failed or not. A failure to create, allow,
/// write or start is the run's error; a failure to remove is a warning the outcome carries.
pub fn run(process: &dyn Process, files: &dyn Files, sbx: &SbxLaunch, command: &ProcessCommand, watch: &dyn LaunchWatch) -> SbxRun {
    let create = sbx_call(process, &sbx.create_args());
    let created_ok = create.is_ok();
    let created = create.and_then(|()| sbx_call(process, &sbx.policy_args()));
    let created = created.and_then(|()| sbx.deny_args().map_or(Ok(()), |args| sbx_call(process, &args)));
    let created = created.and_then(|()| sbx.secret_args().map_or(Ok(()), |args| store_key(process, sbx, &args)));
    let ran = created.and_then(|()| {
        watch.sandbox(&sbx.name);
        watch.note(&format!("sandbox {} (sbx): {} from {}", sbx.name, sbx.program, sbx.image));
        watch.note(&sbx.reach);
        let written = if sbx.env.is_empty() { Ok(()) } else { write_env_file(files, sbx) };
        written.and_then(|()| {
            let exec = ProcessCommand { command: SBX.to_string(), args: sbx.exec_args(), cwd: None, env: BTreeMap::new(), timeout_ms: command.timeout_ms, stdin: command.stdin.clone() };
            process.run_watched(&exec, watch)
        })
    });
    let env_file_warning = if sbx.env.is_empty() { None } else { files.remove(&sbx.env_file).err().map(|error| format!("the sandbox's env file {} was not removed: {}", sbx.env_file, error.message)) };
    if let Ok(launched) = &ran {
        if matches!(launched.result.exit_code, 125 | 127) {
            watch.note(&format!("sbx could not run {} (exit {}); this is sbx's own status, not the client's", sbx.program, launched.result.exit_code));
        }
    }
    let made = created_ok;
    let removed = process.exec(SBX, &["rm", "--force", &sbx.name], Some(SBX_TIMEOUT_MS));
    let mut warnings: Vec<String> = env_file_warning.into_iter().collect();
    warnings.extend(match removed {
        Ok(result) if result.exit_code == 0 => Vec::new(),
        // A sandbox `sbx create` refused was never made: there is nothing left to warn about.
        Ok(_) | Err(_) if !made => Vec::new(),
        Ok(result) => vec![format!("sandbox {} was not removed (sbx rm exited {}); remove it with sbx rm --force {}", sbx.name, result.exit_code, sbx.name)],
        Err(reason) => vec![format!("sandbox {} was not removed ({reason}); remove it with sbx rm --force {}", sbx.name, sbx.name)],
    });
    SbxRun { ran: ran.map_err(|message| format!("sbx run failed: {message}")), warnings }
}

/// The `NAME=value` lines `sbx exec --env-file` reads, written 0600 into the launch's own directory.
fn write_env_file(files: &dyn Files, sbx: &SbxLaunch) -> Result<(), String> {
    let text: String = sbx.env.iter().map(|pair| format!("{pair}\n")).collect();
    files.write_private(&sbx.env_file, &text).map_err(|error| format!("the sandbox's env file was not written: {}", error.message))
}

/// Stores the package proxy's key as the sandbox's custom secret, the key on stdin so no argument list holds it.
fn store_key(process: &dyn Process, sbx: &SbxLaunch, args: &[String]) -> Result<(), String> {
    let Some(credential) = &sbx.credential else { return Ok(()) };
    let command = ProcessCommand { command: SBX.to_string(), args: args.to_vec(), cwd: None, env: BTreeMap::new(), timeout_ms: Some(SBX_TIMEOUT_MS), stdin: Some(credential.key.clone()) };
    match process.run(&command) {
        Ok(ProcessResult { exit_code: 0, timed_out: false, .. }) => Ok(()),
        Ok(result) => Err(format!("sbx secret exited {}: {}", result.exit_code, result.stderr.trim())),
        Err(reason) => Err(format!("sbx secret did not run: {reason}")),
    }
}

/// One `sbx` call that must succeed; its output or its status is the reason when it does not.
fn sbx_call(process: &dyn Process, args: &[String]) -> Result<(), String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match process.exec(SBX, &words, Some(SBX_TIMEOUT_MS)) {
        Ok(ProcessResult { exit_code: 0, timed_out: false, .. }) => Ok(()),
        Ok(result) => Err(format!("sbx {} exited {}: {}", words.first().copied().unwrap_or(""), result.exit_code, result.stderr.trim())),
        Err(reason) => Err(format!("sbx {} did not run: {reason}", words.first().copied().unwrap_or(""))),
    }
}

/// Reads the value of a variable the OpenCode keyed wrapper would have read from the secret store.
pub fn read_key(process: &dyn Process, key: &str) -> Result<String, GatewayError> {
    let value = match process.exec(SECRET_TOOL, &["get", key], Some(SBX_TIMEOUT_MS)) {
        Ok(result) if result.exit_code == 0 => result.stdout.trim().to_string(),
        _ => String::new(),
    };
    if value.is_empty() {
        return Err(GatewayError::new("sandbox-unsupported", format!("the key {key} could not be read from the secret store")));
    }
    Ok(value)
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}
