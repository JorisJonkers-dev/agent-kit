//! The Docker Sandboxes (`sbx`) run of one read-only Pi, OpenCode or Vibe launch, behind `AGENT_KIT_SANDBOX=sbx`.
//!
//! The confined command the launcher would have run is the source: its grants become one sandbox's mounts
//! (`SbxPlan`), its variables pass by name with their values from the command, and its route's base URL decides the
//! one network rule the sandbox gets. A run is `sbx create` (named, from the client's prebuilt image pinned by digest), `sbx policy allow
//! network --sandbox` for the route, `sbx exec` of the client with the watch on its output, and `sbx rm --force`
//! whatever happened before. Everything goes through the `Process` port, so tests use a fake and no real sbx starts.

use crate::error::GatewayError;
use crate::json::{self, Value};
use crate::launchers::{OPENCODE_KEYED, SECRET_TOOL};
use crate::pi_launch::url_port;
use crate::ports::{Files, LaunchWatch, Launched, Process, ProcessCommand, ProcessResult};
use crate::run::RunClient;
use crate::sandbox::{Mount, SbxPlan};
use crate::text::join;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// The program that drives Docker Sandboxes.
const SBX: &str = "sbx";

/// The longest one `sbx` call may take: a cold image pull is about 16 s, a warm create about 2.5 s.
const SBX_TIMEOUT_MS: u64 = 120_000;

/// What the owner runs to get `sbx`, named in every refusal that is about its absence.
pub const SBX_INSTALL: &str = "brew trust docker/tap && brew install docker/tap/sbx, then sign in with sbx login";

/// The host name a sandbox reaches the host's loopback by.
pub(crate) const HOST_FROM_SANDBOX: &str = "host.docker.internal";

/// The client's program in its image, which is also the client's name: Pi, OpenCode and Vibe are the only clients the
/// sbx backend runs so far. Every other client is refused before anything is made.
pub fn program_of(client: RunClient) -> Option<&'static str> {
    match client {
        RunClient::Pi => Some("pi"),
        RunClient::Opencode => Some("opencode"),
        RunClient::Vibe => Some("vibe"),
        _ => None,
    }
}

/// What must hold before a launch makes anything: a client the backend runs and an `sbx` that answers. A writing
/// run is the same launch as a read-only one: its worktree and the git directories it writes are the written mounts
/// of its grants. Refused with the reason and the command that fixes it.
pub fn preflight(process: &dyn Process, client: RunClient) -> Result<(), GatewayError> {
    if program_of(client).is_none() {
        return Err(GatewayError::new("sandbox-unsupported", format!("{}: the sbx backend runs only pi, opencode and vibe so far; use confine or sbx-dry-run for this client", client.as_str())));
    }
    match process.exec(SBX, &["version"], Some(SBX_TIMEOUT_MS)) {
        Ok(result) if result.exit_code == 0 && !result.timed_out => Ok(()),
        Ok(result) => Err(unavailable(&format!("sbx version exited {}", result.exit_code))),
        Err(reason) => Err(unavailable(&reason)),
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
    let listed = match process.exec(SBX, &["ls", "-q"], Some(SBX_TIMEOUT_MS)) {
        Ok(result) if result.exit_code == 0 => result.stdout,
        _ => return Vec::new(),
    };
    let own = std::process::id();
    let mut warnings = Vec::new();
    for name in listed.lines().map(str::trim) {
        let Some(pid) = name.strip_prefix("mg-").and_then(|rest| rest.split('-').next()).and_then(|pid| pid.parse::<u32>().ok()) else {
            continue;
        };
        if pid == own || process.is_alive(pid) {
            continue;
        }
        if !matches!(process.exec(SBX, &["rm", "--force", name], Some(SBX_TIMEOUT_MS)), Ok(result) if result.exit_code == 0) {
            warnings.push(format!("the sandbox {name} of an ended run was not removed; remove it with sbx rm --force {name}"));
        }
    }
    warnings
}

/// The sandbox side of one launch, translated from its confined command and checked.
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
    /// `NAME=value` for each variable the confined launch passes, in the plan's order.
    pub env: Vec<String>,
    /// The hosts the route needs, as `host:port` rules for `sbx policy allow network`.
    pub allow: Vec<String>,
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

    /// `sbx policy allow network --sandbox`, for the route's hosts, in the one rule the sandbox gets.
    pub fn policy_args(&self) -> Vec<String> {
        strings(&["policy", "allow", "network", "--sandbox", &self.name, &self.allow.join(",")])
    }

    /// `sbx exec`: the client in the sandbox, from the plan's workdir, with the variables the launch names read from
    /// the env file, so no value is ever part of the argument list.
    pub fn exec_args(&self) -> Vec<String> {
        let mut args = strings(&["exec", "-i", "-w", &self.plan.workdir]);
        if !self.env.is_empty() {
            args.extend(strings(&["--env-file", &self.env_file]));
        }
        args.push(self.name.clone());
        args.push(self.program.to_string());
        args.extend(self.args.iter().cloned());
        args
    }
}

/// What the sbx side of a launch came to: the client's result (or why it did not start), and what could not be cleaned up.
pub struct SbxRun {
    pub ran: Result<Launched, String>,
    pub warnings: Vec<String>,
}

/// Translates a confined command into its sandbox. `inner` is the confined command itself (`agent-launcher confine
/// ... -- client args`), which OpenCode's keyed wrapper hides behind `/bin/sh`: the wrapper's `env` pairs and its key
/// name are unwrapped here, and the key's value comes from `read_key`, which reads the secret store the wrapper would.
pub fn translate(client: RunClient, name: &str, command: &ProcessCommand, route_text: &str, read_key: &dyn Fn(&str) -> Result<String, GatewayError>) -> Result<SbxLaunch, GatewayError> {
    let program = program_of(client).ok_or_else(|| GatewayError::new("sandbox-unsupported", format!("{}: no sbx program", client.as_str())))?;
    let (inner, wrapped, key) = unwrapped(command);
    let plan = SbxPlan::of_command(client.as_str(), &inner).ok_or_else(|| GatewayError::new("sandbox-unsupported", format!("{}: the launch is not behind agent-launcher confine, so it has no sandbox plan", client.as_str())))?;
    if plan.mounts.iter().all(|mount| mount.read_only) {
        return Err(GatewayError::new("sandbox-unsupported", format!("{}: the sbx plan has no written mount, and sbx needs one as its primary workspace", client.as_str())));
    }
    let gaps: Vec<&String> = plan.gaps.iter().filter(|gap| !gap.starts_with("network:")).collect();
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
    let allow = if client == RunClient::Vibe { crate::sbx_vibe::allowed(route_text, &args)? } else { allowed_hosts(route_text)? };
    Ok(SbxLaunch { name: name.to_string(), program, image: String::new(), env_file: String::new(), plan, args, env, allow })
}

/// The confined command with OpenCode's keyed wrapper taken off: the key's name, and the `env NAME=value` pairs the
/// wrapper sets before the launch, with the launch itself. A command with no wrapper is returned as it is.
fn unwrapped(command: &ProcessCommand) -> (ProcessCommand, BTreeMap<String, String>, Option<String>) {
    let words = &command.args;
    if command.command != "/bin/sh" || words.get(1).map(String::as_str) != Some(OPENCODE_KEYED) {
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
    if client == RunClient::Vibe {
        return crate::sbx_vibe::rewritten(route_text);
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
    let ran = created.and_then(|()| {
        watch.note(&format!("sandbox {} (sbx): {} from {}", sbx.name, sbx.program, sbx.image));
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
