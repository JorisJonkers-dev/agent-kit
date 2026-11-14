//! The Pi critic client: one critic through Pi on a LiteLLM route, in print mode with its JSON event stream, the prompt
//! on stdin and the critic's copy of the packet as its working directory. Pi is started directly, not through the
//! gateway, which has no Pi client. The launch is read-only by its tools: only `read`, `grep`, `find` and `ls`, with
//! every extension, skill, prompt template, theme, context file, MCP server and trust-gated project file switched off,
//! and no session kept. Pi has no sandbox of its own, so the stream is checked afterwards: any other tool, or a path
//! outside the packet, voids the review, and the tripwire catches a write. The node runner's `adapters/pi/index.ts`.

use crate::clients::{ClientReadiness, CriticLaunch, CriticOutcome, CriticReport, CriticTokens};
use crate::critic_tools::PI_TOOLS;
use crate::error::CritiqueError;
use crate::findings::{parse_findings, FindingsParse};
use crate::paths::join;
use crate::routing::PiCriticClient;
use crate::runtime::Runtime;
use crate::vibe_client::{js_string, on_path};
use model_gateway::json::{self, Object, Value};
use model_gateway::ports::{ProcessCommand, ProcessResult};
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

/// Signed in, and the provider offers the routed model, as `pi --list-models` reads `models.json`.
pub fn pi_readiness(runtime: &Runtime, config: &PiCriticClient) -> Result<ClientReadiness, CritiqueError> {
    let sign_in = pi_sign_in(runtime, config)?;
    if !sign_in.ready {
        return Ok(sign_in);
    }
    let listed = runtime.gateway.process.exec(&config.binary, &["--offline", "--list-models", &config.model], Some(STATUS_TIMEOUT_MS));
    Ok(models_readiness(&listed, config))
}

/// One Pi critic's launch: print mode, the JSON stream, the read tools alone, and nothing of the home's or the packet's
/// own. Pi has no time limit of its own, so the outer timer is the limit.
pub fn pi_command(config: &PiCriticClient, launch: &CriticLaunch<'_>, prompt: String) -> ProcessCommand {
    let tools = PI_TOOLS.join(",");
    let args = [
        "-p",
        "--mode",
        "json",
        "--no-session",
        "--offline",
        "--provider",
        &config.provider,
        "--model",
        &config.model,
        "--tools",
        &tools,
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-themes",
        "--no-context-files",
        "--no-mcp",
        "--no-approve",
        "--system-prompt",
        PI_SYSTEM_PROMPT,
    ];
    ProcessCommand {
        command: config.binary.clone(),
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
        cwd: Some(launch.packet_dir.clone()),
        timeout_ms: Some((launch.critic.time_limit_s * 1000.0) as u64),
        stdin: Some(prompt),
        ..ProcessCommand::default()
    }
}

/// Runs one critic through Pi and keeps its answer, price and tokens beside the stream.
pub fn pi_review(runtime: &Runtime, launch: &CriticLaunch<'_>, config: &PiCriticClient) -> Result<CriticReport, CritiqueError> {
    let prompt = runtime.files.read_text(&launch.prompt_path).map_err(CritiqueError::from_fs)?;
    let result = runtime.gateway.process.run(&pi_command(config, launch, prompt)).map_err(CritiqueError::unexpected)?;
    let stream = read_pi_stream(&result.stdout);
    let export_path = join(&[&launch.critic_dir, "result.json"]);
    let number = |value: f64| Value::Num(value);
    let saved = json::object(vec![
        ("answer", stream.answer.as_ref().map_or(Value::Null, |answer| Value::str(answer))),
        ("cost_usd", stream.cost_usd.map_or(Value::Null, number)),
        ("tokens", stream.tokens.as_ref().map_or(Value::Null, |tokens| json::object(vec![("cached", number(tokens.cached)), ("input", number(tokens.input)), ("output", number(tokens.output))]))),
    ]);
    runtime.files.write_text(&export_path, &format!("{}\n", json::pretty(&saved))).map_err(CritiqueError::from_fs)?;
    let outcome = classify_pi_run(&result, &stream, &launch.packet_dir, launch.critic.time_limit_s);
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

/// Pi exits 0 after a provider error too, so the stream decides: the run must end in an assistant message that stopped
/// normally, its tools must be the read tools alone, every path they took must stay in the packet, and its text's last
/// ```json block must match the output contract.
pub fn classify_pi_run(result: &ProcessResult, stream: &PiStream, packet_dir: &str, time_limit_s: f64) -> CriticOutcome {
    let cost = stream.cost_usd;
    let failed = |failure: &str, detail: String| CriticOutcome::Failed { failure: failure.to_string(), detail, cost_usd: cost };
    if result.timed_out {
        return failed("timeout", format!("pi did not return within {}s, so the outer timer killed it", json::number_text(time_limit_s)));
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
    for call in stream.calls.iter().filter(|call| !PI_TOOLS.contains(&call.tool.as_str())) {
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
        Some("error") => return failed("infrastructure-failure", format!("pi stopped on an error{error}")),
        Some("aborted") => return failed("stopped-early", format!("pi was aborted{error}")),
        _ => {}
    }
    match parse_findings(stream.answer.as_deref().unwrap_or("")) {
        FindingsParse::Failed { failure, detail } => failed(failure, detail),
        FindingsParse::Ok { findings, no_findings_reason } => CriticOutcome::Ok { findings, no_findings_reason, cost_usd: cost },
    }
}
