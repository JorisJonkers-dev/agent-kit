//! The Vibe client: what one Vibe run's exit code, journal and export say about it, what it counts against the flow's
//! cap, and the launch that produces it. The node runner's `adapters/vibe/index.ts`.

use crate::clients::{ClientReadiness, CriticLaunch, CriticOutcome, CriticReport, CriticTokens, ModelAnswer, ModelAnswerReport, ModelQuestion};
use crate::findings::{parse_findings, FindingsParse};
use crate::error::CritiqueError;
use crate::pricing::{token_cost, TokenPrices, TokenUsage};
use crate::routing::VibeCriticClient;
use crate::runtime::Runtime;
use model_gateway::client_run::{ClientRun, ClientRunError, ClientRunRequest};
use crate::critic_tools::{CriticToolset, SearchServerLaunch};
use crate::packet_search::SearchToolName;
use crate::vibe_search::{search_failure, tool_calls, vibe_extras};
use model_gateway::json::{number_text, parse, Object, Value};
use model_gateway::launchers::{LaunchPaths, LaunchRequest};
use model_gateway::ports::ProcessResult;
use model_gateway::run::{RunClient, RUN_CLIENTS};
use model_gateway::text::{join, js_trim};
use model_gateway::vibe_sign_in::{node_platform, vibe_signed_in};

/// The tools the runner's search server offers, as `domain/packet-search.ts` names them.
const SEARCH_TOOLS: [&str; 3] = ["grep", "list", "symbol"];

/// The name the runner's search server runs under, as `domain/critic-tools.ts` names it.
const VIBE_SEARCH_SERVER: &str = "packet";

/// What the effective config must show: MCP off, or on with the runner's search server as its only live server.
#[derive(Clone, Debug, PartialEq)]
pub enum ExpectedConfig {
    /// MCP off: the critic reads its packet alone.
    Off,
    /// MCP on, with the runner's search server in this critic's packet copy as the only live server.
    Search { command: String, cwd: String },
}

/// One Vibe launch as its output and export describe it.
#[derive(Clone, Debug, PartialEq)]
pub struct VibeRun {
    pub exit_code: i32,
    pub expected: ExpectedConfig,
    pub stdout: String,
    pub stderr: String,
    /// Parsed `export.json`, or none when Vibe wrote none or it does not parse.
    pub exported: Option<Value>,
    /// The outer timer killed Vibe after `timeout_s` seconds.
    pub timed_out: bool,
    pub timeout_s: f64,
    /// What the run cost: computed from the export's tokens at routing's prices, else Vibe's own figure.
    pub cost_usd: Option<f64>,
    /// The export's `cost_usd`: what Vibe priced the run at by its own model table.
    pub vibe_cost_usd: Option<f64>,
}

/// What a run's final message says: its text, or why there is none.
#[derive(Clone, Debug, PartialEq)]
pub enum VibeMessage {
    Ok { text: String, cost_usd: Option<f64> },
    Failed { failure: String, detail: String, cost_usd: Option<f64> },
}

/// Exit code first: anything but 0 is a failed run, whatever stdout holds. On 0, the export must show an isolated
/// config, and stdout must end in an assistant message, whose text is returned.
pub fn vibe_message(run: &VibeRun) -> VibeMessage {
    let exported = run.exported.as_ref().and_then(Value::as_object);
    let cost = run.cost_usd;
    let failed = |failure: &str, detail: String| VibeMessage::Failed { cost_usd: cost, detail, failure: failure.to_string() };

    if run.timed_out {
        let detail = format!("vibe did not return within {}s, so the outer timer killed it", number_text(run.timeout_s));
        return failed("timeout", detail);
    }
    if run.exit_code != 0 {
        let outcome = match exported.and_then(|export| export.get("outcome")).and_then(Value::as_str) {
            Some(outcome) => format!("outcome {outcome}"),
            None => js_trim(&run.stderr).to_string(),
        };
        return failed(exit_failure(run.exit_code).as_str(), format!("vibe exited {}: {outcome}", run.exit_code));
    }
    if js_trim(&run.stdout).is_empty() {
        return failed("empty-output", "vibe exited 0 with empty stdout".to_string());
    }
    let Some(exported) = exported else {
        return failed("missing-export", "vibe wrote no readable export.json".to_string());
    };
    let violations = isolation_violations(exported.get("config").and_then(Value::as_object), &run.expected);
    if !violations.is_empty() {
        return failed("unsafe-config", format!("effective config: {}", violations.join("; ")));
    }
    match final_assistant_text(&run.stdout) {
        Some(text) => VibeMessage::Ok { cost_usd: cost, text },
        None => failed("unparseable-output", "stdout is not a Vibe journal with an assistant message".to_string()),
    }
}

/// What a Vibe run counts against the flow's cap. The run's cost is the base (computed from its tokens, else Vibe's
/// own figure), and it may exceed the critic's cap: `--max-price` is checked only between turns. Where the export
/// cannot be trusted the cap is charged instead, as a conservative estimate: no export or no cost (a killed run
/// included), a deadline stop (Vibe drops the interrupted turn's tokens and reports $0), and a zero cost for tokens
/// used (a model neither routing nor Vibe prices).
pub fn vibe_charge(run: &VibeRun, cap: f64) -> f64 {
    let exported = run.exported.as_ref().and_then(Value::as_object);
    let Some(cost) = run.cost_usd else {
        return cap;
    };
    if run.timed_out {
        return cap;
    }
    if exported.and_then(|export| export.get("outcome")).and_then(Value::as_str) == Some("deadline") {
        return cost.max(cap);
    }
    let usage = exported.and_then(|export| export.get("usage")).and_then(Value::as_object);
    if cost == 0.0 && used_tokens(usage) {
        cap
    } else {
        cost
    }
}

/// What the run used, from the export's token counts; none when the export counts none.
pub fn tokens_of(exported: Option<&Value>) -> Option<CriticTokens> {
    let usage = usage_of(exported);
    let input = count(usage.and_then(|usage| usage.get("input_tokens")))?;
    let output = count(usage.and_then(|usage| usage.get("output_tokens")))?;
    let cached = count(usage.and_then(|usage| usage.get("cached_input_tokens"))).unwrap_or(0.0);
    Some(CriticTokens { cached, input, output })
}

/// The run's cost at the client's prices, from the export's token counts, or none without prices or counts. Vibe's
/// own `cost_usd` prices cached input at the full rate when its model table has no cached price, which made a
/// cache-heavy critic look three times as costly as Mistral billed.
pub fn computed_cost(exported: Option<&Value>, prices: Option<&TokenPrices>) -> Option<f64> {
    let usage = usage_of(exported);
    let input = count(usage.and_then(|usage| usage.get("input_tokens")))?;
    let output = count(usage.and_then(|usage| usage.get("output_tokens")))?;
    let prices = prices?;
    let cached_input = count(usage.and_then(|usage| usage.get("cached_input_tokens"))).unwrap_or(0.0);
    Some(token_cost(&TokenUsage { cached_input, input, output }, prices))
}

/// The answer a run's message gives: its text when it answered, else why it did not.
pub fn answer_of(message: &VibeMessage) -> ModelAnswer {
    match message {
        VibeMessage::Ok { text, cost_usd } => ModelAnswer::Answered { text: text.clone(), cost_usd: *cost_usd },
        VibeMessage::Failed { failure, detail, cost_usd } => {
            ModelAnswer::Failed { failure: failure.clone(), detail: detail.clone(), cost_usd: *cost_usd }
        }
    }
}

/// The failure an exit code names; a code the client does not document reads `exit-<code>`.
fn exit_failure(code: i32) -> String {
    match code {
        1 => "config-error".to_string(),
        2 => "infrastructure-failure".to_string(),
        3 => "stopped-early".to_string(),
        other => format!("exit-{other}"),
    }
}

/// The export's `usage` object, when the export has one.
fn usage_of(exported: Option<&Value>) -> Option<&Object> {
    exported.and_then(Value::as_object).and_then(|export| export.get("usage")).and_then(Value::as_object)
}

/// A token count the export reports: a finite number at least 0, else none.
fn count(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Num(number)) if number.is_finite() && *number >= 0.0 => Some(*number),
        _ => None,
    }
}

/// Whether the export counts any token at all.
fn used_tokens(usage: Option<&Object>) -> bool {
    ["input_tokens", "output_tokens"]
        .iter()
        .any(|key| matches!(usage.and_then(|usage| usage.get(key)), Some(Value::Num(number)) if *number > 0.0))
}

/// The tools a critic agent may have enabled: `read_file`, and the search server's, which exist only while it runs.
fn agent_tools() -> Vec<String> {
    std::iter::once("read_file".to_string())
        .chain(SEARCH_TOOLS.iter().map(|tool| format!("{VIBE_SEARCH_SERVER}_{tool}")))
        .collect()
}

/// What the effective config gets wrong: MCP off, the agent's tools, no project context, and no way out of the
/// working directory.
fn isolation_violations(config: Option<&Object>, expected: &ExpectedConfig) -> Vec<String> {
    let mut violations = Vec::new();
    let mcp = matches!(expected, ExpectedConfig::Search { .. });
    if config.and_then(|config| config.get("enable_mcp")) != Some(&Value::Bool(mcp)) {
        violations.push(format!("enable_mcp is not {mcp}"));
    }
    if let ExpectedConfig::Search { command, cwd } = expected {
        violations.extend(server_violations(config.and_then(|config| config.get("mcp_servers")), command, cwd));
    }
    let tools = config.and_then(|config| config.get("enabled_tools"));
    let allowed = agent_tools();
    let tools_ok = match tools {
        Some(Value::Arr(items)) => {
            items.iter().any(|tool| tool.as_str() == Some("read_file"))
                && items.iter().all(|tool| tool.as_str().is_some_and(|name| allowed.iter().any(|tool| tool == name)))
        }
        _ => false,
    };
    if !tools_ok {
        violations.push(format!("enabled_tools is not read_file and the search server's tools ({})", allowed.join(", ")));
    }
    if config.and_then(|config| config.get("include_project_context")) != Some(&Value::Bool(false)) {
        violations.push("include_project_context is not false, so the AGENTS.md instructions pointer reaches the critic".to_string());
    }
    if config.and_then(|config| config.get("outside_workdir")).and_then(Value::as_str) != Some("deny") {
        violations.push("outside_workdir is not deny".to_string());
    }
    violations
}

/// With MCP on, the only server not switched off must be the runner's own, in this critic's packet copy.
fn server_violations(servers: Option<&Value>, command: &str, cwd: &str) -> Vec<String> {
    let listed: Vec<Option<&Object>> = match servers {
        Some(Value::Arr(items)) => items.iter().map(Value::as_object).collect(),
        _ => Vec::new(),
    };
    let live: Vec<Option<&Object>> = listed
        .into_iter()
        .filter(|server| server_field(server, "disabled") != Some(&Value::Bool(true)))
        .collect();
    let others: Vec<String> = live
        .iter()
        .filter(|server| server_field(server, "name").and_then(Value::as_str) != Some(VIBE_SEARCH_SERVER))
        .map(|server| js_string(server_field(server, "name")))
        .collect();
    let own: Vec<Option<&Object>> = live
        .iter()
        .filter(|server| server_field(server, "name").and_then(Value::as_str) == Some(VIBE_SEARCH_SERVER))
        .copied()
        .collect();
    let mut violations = Vec::new();
    if !others.is_empty() {
        violations.push(format!("MCP servers other than the search server are on: {}", others.join(", ")));
    }
    let server = own.first().copied().flatten();
    let matches_expected = own.len() == 1
        && server_field(&server, "command").and_then(Value::as_str) == Some(command)
        && server_field(&server, "cwd").and_then(Value::as_str) == Some(cwd);
    if !matches_expected {
        violations.push(format!("the {VIBE_SEARCH_SERVER} MCP server is not the runner's search server in this packet"));
    }
    violations
}

/// One field of one listed MCP server, when the entry is an object that has it.
fn server_field<'a>(server: &Option<&'a Object>, key: &str) -> Option<&'a Value> {
    server.and_then(|server| server.get(key))
}

/// `String(value)` for a JSON value, as the node runner words a server's name.
pub(crate) fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Num(number)) => number_text(*number),
        Some(Value::Str(text)) => text.clone(),
        Some(Value::Arr(items)) => {
            let parts: Vec<String> =
                items.iter().map(|item| if *item == Value::Null { String::new() } else { js_string(Some(item)) }).collect();
            parts.join(",")
        }
        Some(Value::Obj(_)) => "[object Object]".to_string(),
    }
}

/// The journal's entries, each as an object when it is one; none when stdout is not a JSON array.
pub(crate) fn journal_entries(stdout: &str) -> Vec<Option<Object>> {
    match parse(stdout) {
        Ok(Value::Arr(items)) => items.iter().map(|item| item.as_object().cloned()).collect(),
        _ => Vec::new(),
    }
}

/// The text of the journal's last assistant message: its text parts joined, every other part empty.
fn final_assistant_text(stdout: &str) -> Option<String> {
    let entries = journal_entries(stdout);
    let last = entries.iter().rev().find_map(|entry| {
        entry.as_ref().filter(|entry| {
            entry.get("type").and_then(Value::as_str) == Some("message")
                && entry.get("role").and_then(Value::as_str) == Some("assistant")
        })
    })?;
    let content = last.get("content")?;
    let Value::Arr(parts) = content else {
        return None;
    };
    let text = parts
        .iter()
        .map(|part| part.as_object().and_then(|part| part.get("text")).and_then(Value::as_str).unwrap_or(""))
        .collect();
    Some(text)
}

/// Setup's own warning for a personal Vibe home that is not signed in.
pub const SIGN_IN: &str = "vibe: not signed in; run vibe and sign in with your Mistral account, then re-run setup";

/// Seconds the outer timer allows beyond Vibe's own `--time-limit`.
const OUTER_TIMEOUT_GRACE_S: f64 = 60.0;

/// Whether the critic can run on the personal home: `vibe` on `PATH`, the home signed in the way setup checks it
/// (Vibe's item in the OS keyring, else a key in `~/.vibe/.env`; `MISTRAL_API_KEY` does not count, since a critic runs
/// with it unset), and the `critic-<model>` agent setup installs. Nothing is launched, so a client that is not ready
/// falls back in prepare and costs nothing.
pub fn vibe_readiness(runtime: &Runtime, config: &VibeCriticClient) -> Result<ClientReadiness, CritiqueError> {
    let sign_in = vibe_sign_in(runtime, config)?;
    if !sign_in.ready {
        return Ok(sign_in);
    }
    let agent = join(&[&vibe_home(runtime)?, "agents", &format!("{}.toml", config.agent)]);
    if !runtime.files.exists(&agent).unwrap_or(false) {
        let detail = format!("vibe: the critic agent {} is not installed in ~/.vibe/agents; re-run setup", config.agent);
        return Ok(ClientReadiness { ready: false, detail: Some(detail) });
    }
    Ok(ClientReadiness { ready: true, detail: None })
}

/// `vibe` on `PATH` and the personal home signed in, whatever critic agents are installed.
pub fn vibe_sign_in(runtime: &Runtime, config: &VibeCriticClient) -> Result<ClientReadiness, CritiqueError> {
    if !on_path(runtime, &config.binary) {
        return Ok(ClientReadiness { ready: false, detail: Some(format!("vibe: {} is not on PATH", config.binary)) });
    }
    if signed_in(runtime, &vibe_home(runtime)?)? {
        return Ok(ClientReadiness { ready: true, detail: None });
    }
    Ok(ClientReadiness { ready: false, detail: Some(SIGN_IN.to_string()) })
}

/// Puts one question to the agent's model, launched exactly as a critic is (the same isolation, the same checks of the
/// effective config), and returns its final message as plain text.
pub fn vibe_ask(runtime: &Runtime, question: &ModelQuestion, config: &VibeCriticClient) -> Result<ModelAnswerReport, CritiqueError> {
    let paths = LaunchPaths {
        export_dir: crate::paths::join(&[&question.output_dir, "export"]),
        prompt_path: question.prompt_path.clone(),
        run_dir: None,
        work_dir: question.work_dir.clone(),
        git_writable: None,
    };
    let (result, run) = launch_run(runtime, question.time_limit_s, question.max_price_usd, config, &paths, None)?;
    Ok(ModelAnswerReport {
        exit_code: result.exit_code,
        stdout: result.stdout,
        stderr: result.stderr,
        answer: answer_of(&vibe_message(&run)),
        charged_usd: vibe_charge(&run, question.max_price_usd),
        tokens: tokens_of(run.exported.as_ref()),
        vibe_cost_usd: run.vibe_cost_usd,
    })
}

/// Runs one Vibe critic in its packet and reads its review from stdout and the export file, launched exactly as a
/// question is, with the runner's search server in its packet copy when its toolset has one (ADR-0006).
pub fn vibe_review(runtime: &Runtime, launch: &CriticLaunch<'_>, config: &VibeCriticClient) -> Result<CriticReport, CritiqueError> {
    let export_dir = crate::paths::join(&[&launch.critic_dir, "export"]);
    let export_path = crate::paths::join(&[&export_dir, "export.json"]);
    let paths = LaunchPaths { export_dir, prompt_path: launch.prompt_path.clone(), run_dir: None, work_dir: launch.packet_dir.clone(), git_writable: None };
    let search = match &launch.toolset {
        CriticToolset::VibeSearch { search, server, .. } => Some((search.as_slice(), server)),
        _ => None,
    };
    let (result, run) = launch_run(runtime, launch.critic.time_limit_s, launch.critic.max_price_usd, config, &paths, search)?;
    Ok(CriticReport {
        exit_code: result.exit_code,
        stdout: result.stdout,
        stderr: result.stderr,
        export_path,
        outcome: classify_vibe_run(&run),
        charged_usd: vibe_charge(&run, launch.critic.max_price_usd),
        vibe_cost_usd: Some(run.vibe_cost_usd),
        tokens: tokens_of(run.exported.as_ref()),
        search_failure: search.and_then(|_| search_failure(&run.stdout)),
        tool_calls: tool_calls(&run.stdout),
    })
}

/// A critic's run is a review when its final message's last ```json block matches the output contract.
pub fn classify_vibe_run(run: &VibeRun) -> CriticOutcome {
    match vibe_message(run) {
        VibeMessage::Failed { failure, detail, cost_usd } => CriticOutcome::Failed { failure, detail, cost_usd },
        VibeMessage::Ok { text, cost_usd } => match parse_findings(&text) {
            FindingsParse::Failed { failure, detail } => CriticOutcome::Failed { failure: failure.to_string(), detail, cost_usd },
            FindingsParse::Ok { findings, no_findings_reason } => CriticOutcome::Ok { findings, no_findings_reason, cost_usd },
        },
    }
}

/// One Vibe launch through the gateway's launch path, and what its output and export say about it. The gateway builds
/// the command, applies the critic's extras, runs it under its own time and price limits, and reads the export; the
/// runner prices the run and reads its journal itself. Without a search server the effective config must show MCP off.
fn launch_run(
    runtime: &Runtime,
    time_limit_s: f64,
    max_price_usd: f64,
    config: &VibeCriticClient,
    paths: &LaunchPaths,
    search: Option<(&[SearchToolName], &SearchServerLaunch)>,
) -> Result<(ProcessResult, VibeRun), CritiqueError> {
    let request = ClientRunRequest {
        launch: LaunchRequest {
            model: config.gateway.as_ref().map_or_else(|| config.model.clone(), |gateway| gateway.model.clone()),
            time_limit_s,
            max_price_usd: Some(max_price_usd),
            mode: None,
            provider: None,
        },
        binary: config.binary.clone(),
        client: client_of(config)?,
        prompt: runtime.files.read_text(&paths.prompt_path).map_err(CritiqueError::from_fs)?,
        paths: paths.clone(),
        extras: vibe_extras(&config.agent, search),
    };
    let gateway = ClientRun { process: runtime.gateway.process.as_ref(), files: runtime.gateway.files.as_ref(), env: &runtime.gateway.env };
    let launched = gateway.launch(&request).map_err(launch_error)?;
    let exported = launched.exported;
    let vibe_cost_usd = exported.as_ref().and_then(Value::as_object).and_then(|export| export.get("cost_usd")).and_then(number_of);
    let run = VibeRun {
        cost_usd: computed_cost(exported.as_ref(), config.prices.as_ref()).or(vibe_cost_usd),
        exit_code: launched.result.exit_code,
        expected: match search {
            None => ExpectedConfig::Off,
            Some((_, server)) => ExpectedConfig::Search { command: server.command.clone(), cwd: paths.work_dir.clone() },
        },
        exported,
        vibe_cost_usd,
        stderr: launched.result.stderr.clone(),
        stdout: launched.result.stdout.clone(),
        timed_out: launched.result.timed_out,
        timeout_s: time_limit_s + OUTER_TIMEOUT_GRACE_S,
    };
    Ok((launched.result, run))
}

/// Why a launch did not come to an outcome: the gateway's own refusal, or the platform's words for a program that
/// would not start.
pub(crate) fn launch_error(error: ClientRunError) -> CritiqueError {
    match error {
        ClientRunError::Gateway(error) => CritiqueError::unexpected(error.message),
        ClientRunError::Spawn(message) => CritiqueError::unexpected(message),
    }
}

/// The gateway client a critic client runs on: the one its config pins, else Vibe itself. A name the gateway does not
/// run is refused as its own `run` refuses it.
fn client_of(config: &VibeCriticClient) -> Result<RunClient, CritiqueError> {
    gateway_client(config.gateway.as_ref(), "vibe")
}

/// The gateway client a critic client runs on: the one its config pins, else `default`.
pub(crate) fn gateway_client(gateway: Option<&crate::routing::GatewayModel>, default: &str) -> Result<RunClient, CritiqueError> {
    let name = gateway.map_or(default, |gateway| gateway.client.as_str());
    RunClient::parse(name).ok_or_else(|| {
        CritiqueError::new("not-supported-yet", format!("only {} can run a delegation, not {name}", RUN_CLIENTS.join(", ")))
    })
}

/// The personal Vibe home, `~/.vibe`.
pub(crate) fn vibe_home(runtime: &Runtime) -> Result<String, CritiqueError> {
    match runtime.env.get("HOME") {
        Some(home) => Ok(join(&[home, ".vibe"])),
        None => Err(CritiqueError::new("unexpected", "required environment variable is missing: HOME")),
    }
}

/// Whether the home is signed in, asked where Vibe keeps its key.
fn signed_in(runtime: &Runtime, home: &str) -> Result<bool, CritiqueError> {
    vibe_signed_in(runtime.gateway.process.as_ref(), runtime.gateway.files.as_ref(), home, node_platform())
        .map_err(|error| CritiqueError::unexpected(error.message))
}

/// Whether the binary is there: an absolute path as it stands, else the first `PATH` folder that holds it.
pub(crate) fn on_path(runtime: &Runtime, binary: &str) -> bool {
    if binary.starts_with('/') {
        return runtime.files.exists(binary).unwrap_or(false);
    }
    let path = runtime.env.get("PATH").map_or("", String::as_str);
    path.split(':').filter(|dir| !dir.is_empty()).any(|dir| runtime.files.exists(&join(&[dir, binary])).unwrap_or(false))
}

/// A JSON number, and nothing else.
fn number_of(value: &Value) -> Option<f64> {
    match value {
        Value::Num(number) => Some(*number),
        _ => None,
    }
}
