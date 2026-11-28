//! The Pi critic client: one critic through Pi on a LiteLLM route, in print mode with its JSON event stream, the prompt
//! on stdin and the critic's copy of the packet as its working directory. Pi is started directly, not through the
//! gateway, which has no Pi client. The launch is read-only by its tools: only `read`, `grep`, `find` and `ls`, with
//! every extension, skill, prompt template, theme, context file, MCP server and trust-gated project file switched off,
//! and no session kept. Pi has no sandbox of its own and its read tools run inside its own process, so the whole process
//! runs in its own Docker Sandbox (`sbx`, ADR-0019) through the gateway's launch path: it can read only its packet copy
//! and its own agent directory, and reach only the proxy's host. The agent directory is made for the one launch, holds
//! only the route's provider with its key and is removed afterwards. A critic whose sandbox cannot start does not start. The stream is still checked afterwards as a second layer: any other tool, or a
//! path outside the packet, voids the review, and the tripwire catches a write. The node runner's `adapters/pi/index.ts`.

use crate::clients::{ClientReadiness, CriticLaunch, CriticOutcome, CriticReport, CriticTokens};
use crate::critic_tools::{pi_search_tool, CriticToolset, SearchServerLaunch, PI_TOOLS, VIBE_SEARCH_SERVER};
use crate::error::CritiqueError;
use crate::findings::{parse_findings, FindingsParse};
use crate::paths::join;
use crate::routing::PiCriticClient;
use crate::tier_routing::TierRouting;
use crate::tier_review::GATEWAY_CLIENT;
use model_gateway::routing::recorded;
use crate::runtime::Runtime;
use crate::search_server::serve_http;
use crate::vibe_client::{js_string, on_path};
use model_gateway::client_run::{ClientRun, ClientRunError};
use model_gateway::json::{self, Object, Value};
use model_gateway::network::NetworkScope;
use model_gateway::ports::{Launched as Ran, ProcessCommand, ProcessResult};
use model_gateway::run::RunClient;
use model_gateway::sbx_launch;
use model_gateway::sandbox::{SandboxBackend, PLAN_PROGRAM, PLAN_VERB};
use model_gateway::text::js_trim;

/// The longest `pi auth check` and `pi --list-models` may take; they read local files and call no model.
const STATUS_TIMEOUT_MS: u64 = 20_000;

/// Pi's system prompt for a critic, in place of its own, which offers to run commands and points at Pi's documentation
/// outside the packet. The critic's instructions are the prompt itself, on stdin.
pub const PI_SYSTEM_PROMPT: &str =
    "You are a read-only reviewer. Read and search only the files of your working directory, and answer exactly as the user message asks.";

/// `pi` on `PATH` and the route's provider ready: `pi auth check` resolves the provider's key the way a request does
/// without printing it or calling a model.
pub fn pi_sign_in(runtime: &Runtime, config: &PiCriticClient) -> Result<ClientReadiness, CritiqueError> {
    if !on_path(runtime, &config.binary) {
        return Ok(ClientReadiness { ready: false, detail: Some(format!("pi: {} is not on PATH", config.binary)) });
    }
    let checked = runtime.gateway.process.exec(&config.binary, &["auth", "check", "--provider", &config.provider, "--json"], Some(STATUS_TIMEOUT_MS));
    Ok(auth_readiness(&checked, config))
}

/// Signed in, the provider offers the routed model, as `pi --list-models` reads `models.json`, and a sandbox can start.
pub fn pi_readiness(runtime: &Runtime, config: &PiCriticClient) -> Result<ClientReadiness, CritiqueError> {
    let sign_in = pi_sign_in(runtime, config)?;
    if !sign_in.ready {
        return Ok(sign_in);
    }
    // A tier reviewer's home is made for its launch and lists the step model it asks for, so the home's own list says nothing.
    if config.tier.is_none() {
        let listed = runtime.gateway.process.exec(&config.binary, &["--offline", "--list-models", &config.model], Some(STATUS_TIMEOUT_MS));
        let models = models_readiness(&listed, config);
        if !models.ready {
            return Ok(models);
        }
    }
    if let Err(detail) = route(runtime, config) {
        return Ok(ClientReadiness { ready: false, detail: Some(detail) });
    }
    Ok(sandbox_readiness(&ClientRun::sandbox_ready(runtime.gateway.process.as_ref(), &runtime.gateway.env)))
}

/// The exit code `sbx exec` ends with when it could not start the client in its sandbox.
pub const CONFINE_REFUSED: i32 = 125;

/// What a confined launch needs of the route's provider in the home's `models.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct PiRoute {
    pub base_url: String,
    pub api: String,
    pub api_key: String,
    /// The routed model's entry, as the home holds it (its prices included).
    pub model: Value,
    /// The proxy's TCP port, the one port a critic may connect to.
    pub port: u16,
}

/// The port of an `http` or `https` URL: the one after the host, else the scheme's own.
pub fn url_port(url: &str) -> Option<u16> {
    let (rest, default) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(rest), _) => (rest, 443),
        (None, Some(rest)) => (rest, 80),
        (None, None) => return None,
    };
    let authority = rest.split('/').next().unwrap_or("");
    let Some((_, port)) = authority.rsplit_once(':') else {
        return Some(default);
    };
    let digits = !port.is_empty() && port.len() <= 5 && port.bytes().all(|byte| byte.is_ascii_digit());
    port.parse::<u16>().ok().filter(|port| digits && *port > 0)
}

/// The route from the home's provider entry, or why it cannot be used.
pub fn route_of(provider: Option<&Object>, config: &PiCriticClient, path: &str) -> Result<PiRoute, String> {
    let fix = "re-run agent-kit setup so it writes the LiteLLM providers into ~/.pi/agent/models.json";
    let Some(provider) = provider else {
        return Err(format!("pi: {path} has no {} provider; {fix}", config.provider));
    };
    let text = |key: &str| provider.get(key).and_then(Value::as_str).map(str::to_string);
    let (Some(base_url), Some(api), Some(api_key)) = (text("baseUrl"), text("api"), text("apiKey")) else {
        return Err(format!("pi: the {} provider in {path} lacks a baseUrl, api or apiKey; {fix}", config.provider));
    };
    let Some(port) = url_port(&base_url) else {
        return Err(format!("pi: the {} provider's baseUrl is not an http or https URL with a port the confinement can allow", config.provider));
    };
    let entries = match provider.get("models") {
        Some(Value::Arr(models)) => models.clone(),
        _ => Vec::new(),
    };
    let routed = entries.into_iter().find(|entry| entry.as_object().and_then(|entry| entry.get("id")).and_then(Value::as_str) == Some(config.model.as_str()));
    let model = routed.unwrap_or_else(|| json::object(vec![("id", Value::str(&config.model))]));
    Ok(PiRoute { base_url, api, api_key, model, port })
}

/// Whether a sandbox can start here: sbx answers, or why a Pi critic does not run.
pub fn sandbox_readiness(checked: &Result<(), model_gateway::error::GatewayError>) -> ClientReadiness {
    match checked {
        Ok(()) => ClientReadiness { ready: true, detail: None },
        Err(error) => ClientReadiness { ready: false, detail: Some(format!("pi: a Pi critic only runs in a Docker Sandbox, and none can start here: {}", error.message)) },
    }
}

/// The home's Pi directory: `PI_CODING_AGENT_DIR`, else `~/.pi/agent`.
pub(crate) fn home_agent_dir(runtime: &Runtime) -> String {
    match runtime.env.get("PI_CODING_AGENT_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => dir.clone(),
        None => join(&[runtime.env.get("HOME").map_or("", String::as_str), ".pi", "agent"]),
    }
}

/// The home's Pi `bin`, where Pi keeps the `fd` its find tool runs, when there is one.
pub(crate) fn home_bin(runtime: &Runtime) -> Option<String> {
    let bin = join(&[&home_agent_dir(runtime), "bin"]);
    runtime.files.exists(&bin).unwrap_or(false).then_some(bin)
}

/// The route's provider as the home's `models.json` holds it, or why a sandboxed critic cannot use it.
pub(crate) fn route(runtime: &Runtime, config: &PiCriticClient) -> Result<PiRoute, String> {
    route_in(runtime, config, &home_agent_dir(runtime))
}

/// The route's provider as the `models.json` of the Pi directory `dir` holds it.
pub(crate) fn route_in(runtime: &Runtime, config: &PiCriticClient, dir: &str) -> Result<PiRoute, String> {
    let path = join(&[dir, "models.json"]);
    let text = runtime.files.read_text(&path).ok();
    let providers = text.as_deref().and_then(parse_object).and_then(|models| models.get("providers").and_then(Value::as_object).cloned());
    route_of(providers.as_ref().and_then(|providers| providers.get(&config.provider)).and_then(Value::as_object), config, &path)
}

/// The key: a literal as it is, or `!<program> get <NAME>` run without a shell; never a shell command.
pub(crate) fn key(runtime: &Runtime, config: &PiCriticClient, api_key: &str) -> Result<String, String> {
    let Some(command) = api_key.strip_prefix('!') else {
        return Ok(api_key.to_string());
    };
    let words: Vec<&str> = command.split_whitespace().collect();
    let [program, "get", name] = words.as_slice() else {
        return Err(format!("pi: the {} key is a shell command the runner does not run; setup writes it as !agent-kit-secret get <NAME>", config.provider));
    };
    let read = runtime.gateway.process.exec(program, &["get", name], Some(STATUS_TIMEOUT_MS));
    let value = match &read {
        Ok(result) if !result.timed_out && result.exit_code == 0 => js_trim(&result.stdout).to_string(),
        _ => String::new(),
    };
    if !value.is_empty() {
        return Ok(value);
    }
    // Only the error stream is shown: a failed read never puts anything of the key in a message.
    let said = match &read {
        Err(reason) => reason.clone(),
        Ok(result) => first_line(&result.stderr).unwrap_or_else(|| format!("exit {}", result.exit_code)),
    };
    Err(format!("pi: the {} key could not be read from the secret store ({name}): {said}", config.provider))
}

/// The grants of a sandboxed launch: the packet and the agent directory when there is a launch, Pi, its tools and the
/// proxy's port.
pub(crate) fn grants(runtime: &Runtime, config: &PiCriticClient, route: &PiRoute, launch: Option<(&str, &str)>, server: Option<&SearchAccess>) -> Vec<String> {
    let mut grants = Vec::new();
    if let Some((packet_dir, _)) = launch {
        grants.extend(["--read".to_string(), packet_dir.to_string()]);
    }
    grants.extend(["--run-from".to_string(), config.binary.clone()]);
    if let Some(SearchAccess::Program(server)) = server {
        // Pi starts the search server itself, inside the confinement: it runs from its own program, and reads the
        // files its arguments name (a script it is started with), the packet being granted already.
        grants.extend(["--run-from".to_string(), server.command.clone()]);
        for argument in server.args.iter().filter(|argument| argument.starts_with('/') && runtime.files.exists(argument).unwrap_or(false)) {
            grants.extend(["--read".to_string(), argument.clone()]);
        }
    }
    if let Some(bin) = home_bin(runtime) {
        grants.extend(["--run".to_string(), bin]);
    }
    if let Some((_, agent_dir)) = launch {
        grants.extend(["--write".to_string(), agent_dir.to_string()]);
    }
    grants.extend(["--net-connect".to_string(), route.port.to_string()]);
    // The sandbox is handed no more than the variables named here; Pi's agent directory is the one it names.
    grants.extend(["--env".to_string(), "PI_CODING_AGENT_DIR".to_string()]);
    grants
}

/// The launch's own agent directory: a `models.json` holding only the route's provider, its key read from the secret
/// store just now and written in plain (mode 0600), and an empty `settings.json`; the route, or why not.
pub(crate) fn prepare(runtime: &Runtime, config: &PiCriticClient, agent_dir: &str, server: Option<&SearchAccess>) -> Result<Result<PiRoute, String>, CritiqueError> {
    let route = match route(runtime, config) {
        Ok(route) => route,
        Err(detail) => return Ok(Err(detail)),
    };
    let key = match key(runtime, config, &route.api_key) {
        Ok(key) => key,
        Err(detail) => return Ok(Err(detail)),
    };
    write_home(runtime, config, agent_dir, (&route, &key), server)?;
    Ok(Ok(route))
}

/// The files of a launch's own agent directory: a `models.json` holding only the route's provider with the key given (the
/// real key, or the token of the recording proxy) and the one model of the launch, an empty `settings.json`, and the search
/// server's `mcp.json` when the critic has one. The key goes in last, so a write that fails leaves no key behind.
fn write_home(runtime: &Runtime, config: &PiCriticClient, agent_dir: &str, (route, key): (&PiRoute, &str), server: Option<&SearchAccess>) -> Result<(), CritiqueError> {
    let provider = json::object(vec![
        ("api", Value::str(&route.api)),
        ("apiKey", Value::str(key)),
        ("baseUrl", Value::str(&route.base_url)),
        ("models", Value::Arr(vec![route.model.clone()])),
    ]);
    let models = json::object(vec![("providers", json::object(vec![(config.provider.as_str(), provider)]))]);
    runtime.files.write_private(&join(&[agent_dir, "settings.json"]), "{}\n").map_err(CritiqueError::from_fs)?;
    if let Some(server) = server {
        runtime.files.write_private(&join(&[agent_dir, "mcp.json"]), &format!("{}\n", json::pretty(&server.mcp_servers()))).map_err(CritiqueError::from_fs)?;
    }
    runtime.files.write_private(&join(&[agent_dir, "models.json"]), &format!("{}\n", json::pretty(&models))).map_err(CritiqueError::from_fs)
}

/// How a Pi critic reaches the runner's search server: by starting it (a confined launch on the host) or by calling the
/// one the runner serves on the host over HTTP, which is how a critic in a Docker Sandbox reaches it, since no
/// sandbox image holds the program (ADR-0006, amended).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchAccess<'a> {
    Program(&'a SearchServerLaunch),
    Url(String),
}

impl SearchAccess<'_> {
    /// The launch's `mcp.json` for this way of reaching the server.
    pub fn mcp_servers(&self) -> Value {
        match self {
            SearchAccess::Program(server) => mcp_servers(server),
            SearchAccess::Url(url) => mcp_entry(vec![("url", Value::str(url))]),
        }
    }
}

/// The launch's `mcp.json`: the runner's search server as the only MCP server, its tools called directly by name. Pi
/// reads MCP servers from its configuration alone, and the launch's agent directory is the one it reads.
pub fn mcp_servers(server: &SearchServerLaunch) -> Value {
    let args = server.args.iter().map(|argument| Value::str(argument)).collect();
    mcp_entry(vec![("command", Value::str(&server.command)), ("args", Value::Arr(args))])
}

/// `mcp.json` with the search server as its one entry: how it is reached, then how its tools are shown.
fn mcp_entry(reach: Vec<(&str, Value)>) -> Value {
    let mut pairs = reach;
    pairs.push(("exposure", Value::str("direct")));
    pairs.push(("description", Value::str("Search the files of your working directory")));
    json::object(vec![("mcpServers", json::object(vec![(VIBE_SEARCH_SERVER, json::object(pairs))]))])
}

/// The Pi launch as a plan command, which the gateway reads and runs in a sandbox: the grants, then `--` and Pi's own
/// command line. Pi reads its agent directory from `PI_CODING_AGENT_DIR`, keeps its temporary files there, and finds the home's `fd` on
/// `PATH`.
pub fn confined_command(grants: Vec<String>, pi: ProcessCommand, path: Option<&str>, agent_dir: &str, bin: Option<&str>) -> ProcessCommand {
    let searched = match (bin, path) {
        (None, path) => path.unwrap_or("").to_string(),
        (Some(bin), Some(path)) if !path.is_empty() => format!("{bin}:{path}"),
        (Some(bin), _) => bin.to_string(),
    };
    let mut args = vec![PLAN_VERB.to_string()];
    args.extend(grants);
    args.push("--".to_string());
    args.push(pi.command.clone());
    args.extend(pi.args.iter().cloned());
    let env = [("PATH", searched), ("PI_CODING_AGENT_DIR", agent_dir.to_string()), ("TMPDIR", agent_dir.to_string())]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    ProcessCommand { command: PLAN_PROGRAM.to_string(), args, env, ..pi }
}

/// A Pi critic that never started because no sandbox could be made for it.
pub(crate) fn unconfined_report(launch: &CriticLaunch<'_>, detail: String) -> CriticReport {
    CriticReport {
        exit_code: CONFINE_REFUSED,
        stdout: String::new(),
        stderr: String::new(),
        export_path: join(&[&launch.critic_dir, "result.json"]),
        outcome: CriticOutcome::Failed { failure: "unconfined".to_string(), detail, cost_usd: None },
        charged_usd: 0.0,
        vibe_cost_usd: None,
        tokens: None,
        search_failure: None,
        tool_calls: Vec::new(),
        routing: None,
    }
}

/// One Pi critic's launch: print mode, the JSON stream, the read tools alone, and nothing of the home's or the packet's
/// own. Pi has no time limit of its own, so the outer timer is the limit.
pub fn pi_command(config: &PiCriticClient, launch: &CriticLaunch<'_>, prompt: String) -> ProcessCommand {
    let searching = matches!(launch.toolset, CriticToolset::PiSearch { .. });
    // With the search server on, `--tools` names its tools too (an `mcp__` entry filters MCP tools as well). MCP is a
    // built-in extension, which `--no-extensions` switches off, so it is named back in; without the server MCP is off.
    let tools = if searching { format!("{},mcp__{VIBE_SEARCH_SERVER}__*", PI_TOOLS.join(",")) } else { PI_TOOLS.join(",") };
    let mcp: &[&str] = if searching { &["-e", "builtin:mcp"] } else { &["--no-mcp"] };
    let args: Vec<&str> = [
        &["-p", "--mode", "json", "--no-session", "--offline", "--provider", &config.provider, "--model", &config.model, "--tools", &tools][..],
        &["--no-extensions"],
        if searching { mcp } else { &[] },
        &["--no-skills", "--no-prompt-templates", "--no-themes", "--no-context-files"],
        if searching { &[] } else { mcp },
        &["--no-approve", "--system-prompt", PI_SYSTEM_PROMPT],
    ]
    .concat();
    ProcessCommand {
        command: config.binary.clone(),
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
        cwd: Some(launch.packet_dir.clone()),
        timeout_ms: Some((launch.critic.time_limit_s * 1000.0) as u64),
        stdin: Some(prompt),
        ..ProcessCommand::default()
    }
}

/// The plan command run in a sandbox through the gateway's launch path, the one every worker takes: the same backend
/// choice, `sbx` preflight, image and cleanup. A refusal (no working sbx) is the reason nothing ran.
fn run_in_sandbox(runtime: &Runtime, command: &ProcessCommand, agent_dir: &str, server: Option<&SearchAccess>) -> Launched {
    let gateway = ClientRun { process: runtime.gateway.process.as_ref(), files: runtime.gateway.files.as_ref(), clock: runtime.clock.as_ref(), env: &runtime.gateway.env };
    // A critic that searches over HTTP is allowed that one host and port, as the route's proxy is.
    let allow = match server {
        Some(SearchAccess::Url(url)) => match sbx_launch::host_rules(&url.replacen(sbx_launch::HOST_FROM_SANDBOX, "127.0.0.1", 1)) {
            Ok(rules) => rules,
            Err(error) => return Launched::Refused(error.message),
        },
        _ => Vec::new(),
    };
    match gateway.launch_planned_allowing(RunClient::Pi, command, agent_dir, &NetworkScope::default(), &allow) {
        Ok(Ran { result, .. }) => Launched::Ran(Ok(result)),
        Err(ClientRunError::Gateway(error)) => Launched::Refused(error.message),
        Err(ClientRunError::Spawn(message)) => Launched::Ran(Err(message)),
    }
}

/// What one launch left: the process (or why it never started), and for a tier reviewer what LiteLLM said of it.
enum Launched {
    /// The critic could not be sandboxed or set up, and nothing ran.
    Refused(String),
    Ran(Result<ProcessResult, String>),
}

/// The launch in its sandbox, through the route's key as the home holds it.
fn launch_direct(runtime: &Runtime, launch: &CriticLaunch<'_>, config: &PiCriticClient, agent_dir: &str, prompt: String, server: Option<&SearchAccess>) -> Result<Launched, CritiqueError> {
    let route = match prepare(runtime, config, agent_dir, server)? {
        Ok(route) => route,
        Err(detail) => return Ok(Launched::Refused(detail)),
    };
    let grants = grants(runtime, config, &route, Some((&launch.packet_dir, agent_dir)), server);
    let bin = home_bin(runtime);
    let command = confined_command(grants, pi_command(config, launch, prompt), runtime.env.get("PATH").map(String::as_str), agent_dir, bin.as_deref());
    let launched = run_in_sandbox(runtime, &command, agent_dir, server);
    let _ = runtime.files.remove(agent_dir);
    Ok(launched)
}

/// The launch in its sandbox, in the per-run home the recording proxy made: its provider's base URL is the proxy and its key is
/// the run's token, so the real key never reaches the critic. The home is written again to list only the step model the
/// reviewer asks for.
fn launch_proxied(runtime: &Runtime, launch: &CriticLaunch<'_>, config: &PiCriticClient, agent_dir: &str, prompt: String, server: Option<&SearchAccess>) -> Result<Launched, CritiqueError> {
    let mut route = route_in(runtime, config, agent_dir).map_err(CritiqueError::unexpected)?;
    route.model = json::object(vec![("id", Value::str(&config.model))]);
    write_home(runtime, config, agent_dir, (&route, &route.api_key), server)?;
    let grants = grants(runtime, config, &route, Some((&launch.packet_dir, agent_dir)), server);
    let bin = home_bin(runtime);
    let command = confined_command(grants, pi_command(config, launch, prompt), runtime.env.get("PATH").map(String::as_str), agent_dir, bin.as_deref());
    Ok(run_in_sandbox(runtime, &command, agent_dir, server))
}

/// A tier reviewer's launch behind the recording proxy: the launch and what LiteLLM's headers said of it. When the proxy
/// cannot be set up the launch runs as any Pi critic does, and its routing is `hidden`, which does not prove its family.
fn launch_recorded(runtime: &Runtime, launch: &CriticLaunch<'_>, config: &PiCriticClient, tier: &str, agent_dir: &str, prompt: String, server: Option<&SearchAccess>) -> Result<(Launched, TierRouting), CritiqueError> {
    let mut launched: Option<Result<Launched, CritiqueError>> = None;
    let routing = recorded(&runtime.gateway, "curl", agent_dir, &config.provider, GATEWAY_CLIENT, &mut |env| {
        let proxied = env.get("PI_CODING_AGENT_DIR").map(String::as_str) == Some(agent_dir);
        launched = Some(if proxied { launch_proxied(runtime, launch, config, agent_dir, prompt.clone(), server) } else { launch_direct(runtime, launch, config, agent_dir, prompt.clone(), server) });
    });
    let launched = launched.expect("the launch ran, with or without the proxy")?;
    Ok((launched, TierRouting::of(tier, &config.model, &routing)))
}

/// Runs one critic through Pi and keeps its answer, price and tokens beside the stream. A tier reviewer's run goes through
/// the recording proxy, and counts as a review only when the routing shows the step model it asked for answered.
pub fn pi_review(runtime: &Runtime, launch: &CriticLaunch<'_>, config: &PiCriticClient) -> Result<CriticReport, CritiqueError> {
    let prompt = runtime.files.read_text(&launch.prompt_path).map_err(CritiqueError::from_fs)?;
    let agent_dir = join(&[&launch.critic_dir, "pi-agent"]);
    let program = match &launch.toolset {
        CriticToolset::PiSearch { server, .. } => Some(server),
        _ => None,
    };
    // No sandbox image holds the search server's program, so a critic in a Docker Sandbox calls the runner's own server
    // over HTTP: it listens on the host's loopback for the length of the critic and the sandbox is allowed that one port.
    let in_sandbox = !SandboxBackend::select(&runtime.gateway.env).is_ok_and(SandboxBackend::is_host);
    let (http, access) = match (program, &launch.toolset) {
        (Some(_), CriticToolset::PiSearch { limits, .. }) if in_sandbox => match serve_http(&launch.packet_dir, limits.clone()) {
            Ok(http) => {
                let url = format!("http://{}:{}/mcp", sbx_launch::HOST_FROM_SANDBOX, http.port());
                (Some(http), Some(SearchAccess::Url(url)))
            }
            Err(detail) => return Ok(unconfined_report(launch, format!("pi: the packet search server could not start on the host: {detail}"))),
        },
        (Some(server), _) => (None, Some(SearchAccess::Program(server))),
        _ => (None, None),
    };
    let server = access.as_ref();
    let (launched, routing) = match &config.tier {
        Some(tier) => {
            let (launched, routing) = launch_recorded(runtime, launch, config, tier, &agent_dir, prompt, server)?;
            (launched, Some(routing))
        }
        None => (launch_direct(runtime, launch, config, &agent_dir, prompt, server)?, None),
    };
    let ran = match launched {
        Launched::Refused(detail) => return Ok(CriticReport { routing, ..unconfined_report(launch, detail) }),
        Launched::Ran(ran) => ran,
    };
    drop(http);
    let result = ran.map_err(CritiqueError::unexpected)?;
    let stream = read_pi_stream(&result.stdout);
    let export_path = join(&[&launch.critic_dir, "result.json"]);
    let number = |value: f64| Value::Num(value);
    let saved = json::object(vec![
        ("answer", stream.answer.as_ref().map_or(Value::Null, |answer| Value::str(answer))),
        ("cost_usd", stream.cost_usd.map_or(Value::Null, number)),
        ("tokens", stream.tokens.as_ref().map_or(Value::Null, |tokens| json::object(vec![("cached", number(tokens.cached)), ("input", number(tokens.input)), ("output", number(tokens.output))]))),
    ]);
    runtime.files.write_text(&export_path, &format!("{}\n", json::pretty(&saved))).map_err(CritiqueError::from_fs)?;
    let mut outcome = classify_pi_run(&result, &stream, &launch.packet_dir, launch.critic.time_limit_s, &allowed_tools(&launch.toolset));
    if let (CriticOutcome::Ok { cost_usd, .. }, Some((failure, detail))) = (&outcome, routing.as_ref().and_then(TierRouting::collapse)) {
        outcome = CriticOutcome::Failed { failure: failure.to_string(), detail, cost_usd: *cost_usd };
    }
    Ok(CriticReport {
        exit_code: result.exit_code,
        stdout: result.stdout,
        stderr: result.stderr,
        export_path,
        outcome,
        // Pi spends on its LiteLLM route's budget, never the flow's Vibe cap; the gateway ledger carries what Pi priced it at.
        charged_usd: stream.cost_usd.unwrap_or(0.0),
        vibe_cost_usd: None,
        tokens: stream.tokens,
        search_failure: None,
        tool_calls: Vec::new(),
        routing,
    })
}

/// The first non-blank line of a text.
fn first_line(text: &str) -> Option<String> {
    text.split('\n').map(|line| js_trim(line).to_string()).find(|line| !line.is_empty())
}

/// Why a status command could not say, as one clause.
fn unreadable(result: &Result<ProcessResult, String>) -> String {
    match result {
        Err(reason) => reason.clone(),
        Ok(result) if result.timed_out => format!("no answer within {}s", STATUS_TIMEOUT_MS / 1000),
        Ok(result) => first_line(&format!("{}\n{}", result.stdout, result.stderr)).unwrap_or_else(|| format!("exit {}", result.exit_code)),
    }
}

/// A JSON object, or none for any other value or text that does not parse.
fn parse_object(text: &str) -> Option<Object> {
    json::parse(text).ok().and_then(|value| value.as_object().cloned())
}

/// What `pi auth check --json` says: `{"status": "ready" | "not_ready" | "invalid", "reason"?}`.
pub fn auth_readiness(checked: &Result<ProcessResult, String>, config: &PiCriticClient) -> ClientReadiness {
    let failed = || ClientReadiness { ready: false, detail: Some(format!("pi: auth check for {} failed: {}", config.provider, unreadable(checked))) };
    let result = match checked {
        Ok(result) if !result.timed_out => result,
        _ => return failed(),
    };
    let answer = parse_object(js_trim(&result.stdout));
    let Some(status) = answer.as_ref().and_then(|answer| answer.get("status")).and_then(Value::as_str) else {
        return failed();
    };
    if status == "ready" {
        return ClientReadiness { ready: true, detail: None };
    }
    let reason = answer.as_ref().and_then(|answer| answer.get("reason")).and_then(Value::as_str).map_or_else(String::new, |reason| format!(" ({reason})"));
    ClientReadiness {
        ready: false,
        detail: Some(format!(
            "pi: the {} provider is {status}{reason}; re-run agent-kit setup so it writes the LiteLLM providers into ~/.pi/agent/models.json",
            config.provider
        )),
    }
}

/// Whether `pi --list-models <model>` lists the routed model under the route's provider: its first two columns.
pub fn models_readiness(listed: &Result<ProcessResult, String>, config: &PiCriticClient) -> ClientReadiness {
    let result = match listed {
        Ok(result) if !result.timed_out && result.exit_code == 0 => result,
        _ => return ClientReadiness { ready: false, detail: Some(format!("pi: {} --list-models failed: {}", config.binary, unreadable(listed))) },
    };
    let offered = result.stdout.split('\n').any(|line| {
        let mut columns = line.split_whitespace();
        columns.next() == Some(config.provider.as_str()) && columns.next() == Some(config.model.as_str())
    });
    if offered {
        return ClientReadiness { ready: true, detail: None };
    }
    ClientReadiness { ready: false, detail: Some(format!("pi: the {} provider does not offer {}", config.provider, config.model)) }
}

/// One tool call of the stream: its tool and its arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct PiToolCall {
    pub tool: String,
    pub args: Option<Object>,
}

/// What Pi's JSON event stream held.
#[derive(Clone, Debug, PartialEq)]
pub struct PiStream {
    /// False when a line of stdout is not a JSON object.
    pub parsed: bool,
    pub calls: Vec<PiToolCall>,
    /// The last assistant message, which ends the run.
    pub last: Option<Object>,
    /// The last assistant message's text.
    pub answer: Option<String>,
    /// What Pi priced every assistant message at, summed; none when none carried a price.
    pub cost_usd: Option<f64>,
    pub tokens: Option<CriticTokens>,
}

/// A finite number of at least 0, and nothing else.
fn non_negative(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Num(number)) if number.is_finite() && *number >= 0.0 => Some(*number),
        _ => None,
    }
}

/// Reads the stream line by line: each tool call's start, and each assistant message's end with its usage. Pi's `input`
/// leaves out the cached prompt, which it counts as `cacheRead`, so the input recorded is both.
pub fn read_pi_stream(stdout: &str) -> PiStream {
    let mut stream = PiStream { parsed: true, calls: Vec::new(), last: None, answer: None, cost_usd: None, tokens: None };
    for line in stdout.split('\n').filter(|text| !js_trim(text).is_empty()) {
        let Some(event) = parse_object(line) else {
            return PiStream { parsed: false, last: None, answer: None, ..stream };
        };
        let kind = event.get("type").and_then(Value::as_str);
        if kind == Some("tool_execution_start") {
            stream.calls.push(PiToolCall { tool: js_string(event.get("toolName")), args: event.get("args").and_then(Value::as_object).cloned() });
        }
        let message = if kind == Some("message_end") { event.get("message").and_then(Value::as_object) } else { None };
        let Some(message) = message.filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant")) else {
            continue;
        };
        stream.last = Some(message.clone());
        let usage = message.get("usage").and_then(Value::as_object);
        let field = |key: &str| usage.and_then(|usage| usage.get(key));
        if let Some(price) = non_negative(field("cost").and_then(Value::as_object).and_then(|cost| cost.get("total"))) {
            stream.cost_usd = Some(stream.cost_usd.unwrap_or(0.0) + price);
        }
        let (Some(input), Some(output)) = (non_negative(field("input")), non_negative(field("output"))) else {
            continue;
        };
        let cached = non_negative(field("cacheRead")).unwrap_or(0.0);
        let before = stream.tokens.take().unwrap_or(CriticTokens { cached: 0.0, input: 0.0, output: 0.0 });
        stream.tokens = Some(CriticTokens { cached: before.cached + cached, input: before.input + input + cached, output: before.output + output });
    }
    stream.answer = stream.last.as_ref().map(message_text);
    stream
}

/// The text parts of a message, joined.
fn message_text(message: &Object) -> String {
    let Some(Value::Arr(content)) = message.get("content") else {
        return String::new();
    };
    content
        .iter()
        .filter_map(Value::as_object)
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect()
}

/// The segments of `path` once `.` and `..` are resolved lexically; none when a `..` climbs above where it starts.
fn resolved_segments(path: &str) -> Option<Vec<&str>> {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            _ => segments.push(segment),
        }
    }
    Some(segments)
}

/// Whether a path a tool was given reaches outside the packet: Pi resolves a relative path against its working
/// directory, takes an absolute one as it is, expands `~`, and drops a leading `@`.
pub fn outside_packet(raw: &str, packet_dir: &str) -> bool {
    let path = raw.strip_prefix('@').unwrap_or(raw);
    if path.starts_with('~') {
        return true;
    }
    if !path.starts_with('/') {
        return resolved_segments(path).is_none();
    }
    let root: Vec<&str> = packet_dir.split('/').filter(|segment| !segment.is_empty()).collect();
    let segments = resolved_segments(path).unwrap_or_default();
    segments.len() < root.len() || root.iter().zip(&segments).any(|(own, other)| own != other)
}

/// Whether a glob a tool was given can match outside the packet: an absolute or home glob, or one that climbs with `..`.
fn glob_outside(glob: &str) -> bool {
    glob.starts_with('/') || glob.starts_with('~') || glob.split('/').any(|segment| segment == "..")
}

/// Every call that reached outside the packet, as `<tool> <argument>`, in order and once each.
pub fn outside_calls(calls: &[PiToolCall], packet_dir: &str) -> Vec<String> {
    let mut outside: Vec<String> = Vec::new();
    for call in calls {
        let arg = |key: &str| call.args.as_ref().and_then(|args| args.get(key)).and_then(Value::as_str);
        let mut reached: Vec<&str> = Vec::new();
        reached.extend(arg("path").filter(|path| outside_packet(path, packet_dir)));
        let pattern = if call.tool == "find" { arg("pattern") } else { None };
        reached.extend([arg("glob"), pattern].into_iter().flatten().filter(|glob| glob_outside(glob)));
        for argument in reached {
            let shown = format!("{} {argument}", call.tool);
            if !outside.contains(&shown) {
                outside.push(shown);
            }
        }
    }
    outside
}

/// The tools a Pi critic may call: the read tools, and the search server's when it has them.
pub fn allowed_tools(toolset: &CriticToolset) -> Vec<String> {
    let mut allowed: Vec<String> = PI_TOOLS.iter().map(|tool| (*tool).to_string()).collect();
    if let CriticToolset::PiSearch { search, .. } = toolset {
        allowed.extend(search.iter().map(|tool| pi_search_tool(*tool)));
    }
    allowed
}

/// Whether a provider's error message says the model is rate-limited or out of quota: the gateway's ledger then holds the
/// model as cooled, and a tier review leaves it for another step.
pub(crate) fn is_rate_limit(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    ["429", "rate limit", "rate-limit", "quota"].iter().any(|word| lower.contains(word))
}

/// Pi exits 0 after a provider error too, so the stream decides: the run must end in an assistant message that stopped
/// normally, its tools must be the read tools alone, every path they took must stay in the packet, and its text's last
/// ```json block must match the output contract.
pub fn classify_pi_run(result: &ProcessResult, stream: &PiStream, packet_dir: &str, time_limit_s: f64, allowed: &[String]) -> CriticOutcome {
    let cost = stream.cost_usd;
    let failed = |failure: &str, detail: String| CriticOutcome::Failed { failure: failure.to_string(), detail, cost_usd: cost };
    if result.timed_out {
        return failed("timeout", format!("pi did not return within {}s, so the outer timer killed it", json::number_text(time_limit_s)));
    }
    if result.exit_code == CONFINE_REFUSED {
        let said = first_line(&result.stderr).unwrap_or_else(|| "exit 125".to_string());
        return failed("unconfined", format!("sbx could not start pi in a sandbox, so it did not start: {said}"));
    }
    if result.exit_code != 0 {
        let said = first_line(&result.stderr).map_or_else(String::new, |said| format!(": {said}"));
        return failed(&format!("exit-{}", result.exit_code), format!("pi exited {}{said}", result.exit_code));
    }
    if js_trim(&result.stdout).is_empty() {
        return failed("empty-output", "pi exited 0 with empty stdout".to_string());
    }
    if !stream.parsed {
        return failed("unparseable-output", "stdout is not a Pi JSON event stream".to_string());
    }
    let mut unsafe_tools: Vec<&str> = Vec::new();
    for call in stream.calls.iter().filter(|call| !allowed.contains(&call.tool)) {
        if !unsafe_tools.contains(&call.tool.as_str()) {
            unsafe_tools.push(&call.tool);
        }
    }
    if !unsafe_tools.is_empty() {
        return failed("unsafe-tool", format!("the critic used tools beyond reading its packet: {}", unsafe_tools.join(", ")));
    }
    let outside = outside_calls(&stream.calls, packet_dir);
    if !outside.is_empty() {
        return failed("unsafe-tool", format!("the critic reached outside its packet: {}", outside.join(", ")));
    }
    let Some(last) = &stream.last else {
        return failed("unparseable-output", "stdout is not a Pi event stream with an assistant message".to_string());
    };
    let error = last.get("errorMessage").and_then(Value::as_str).map_or_else(String::new, |message| format!(": {message}"));
    match last.get("stopReason").and_then(Value::as_str) {
        Some("error") if is_rate_limit(&error) => return failed("rate-limited", format!("pi stopped on a rate limit{error}")),
        Some("error") => return failed("infrastructure-failure", format!("pi stopped on an error{error}")),
        Some("aborted") => return failed("stopped-early", format!("pi was aborted{error}")),
        _ => {}
    }
    match parse_findings(stream.answer.as_deref().unwrap_or("")) {
        FindingsParse::Failed { failure, detail } => failed(failure, detail),
        FindingsParse::Ok { findings, no_findings_reason } => CriticOutcome::Ok { findings, no_findings_reason, cost_usd: cost },
    }
}
