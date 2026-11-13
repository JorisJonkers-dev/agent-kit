//! The Antigravity critic client: one Gemini critic through `agy` in print mode, its review read from the `stream-json`
//! events. The node runner's `adapters/agy/index.ts`.

use crate::clients::{ClientReadiness, CriticLaunch, CriticOutcome, CriticReport};
use crate::critic_tools::tool_names;
use crate::error::CritiqueError;
use crate::findings::{read_findings, FindingsParse};
use crate::paths::join;
use crate::routing::AgyCriticClient;
use crate::runtime::Runtime;
use crate::vibe_client::{gateway_client, js_string, launch_error, on_path};
use model_gateway::client_run::{ClientRun, ClientRunRequest};
use model_gateway::extras::{RunExtras, SandboxProfile};
use model_gateway::json::{self, Object, Value};
use model_gateway::launchers::{LaunchPaths, LaunchRequest};
use model_gateway::ports::ProcessResult;
use model_gateway::text::js_trim;

/// Setup's own warning for a machine where `agy models` asks for a sign-in.
const SIGN_IN: &str = "antigravity: needs sign-in: run agy and sign in with your Google account";

/// The longest `agy models` may take. Signed in it answers in about a second; stdin is closed, so it cannot wait on a
/// prompt either.
const MODELS_TIMEOUT_MS: u64 = 20_000;

/// The progress line `agy models` prints first, which says nothing about the outcome.
const MODELS_PROGRESS: &str = "Fetching available models...";

/// Seconds the outer timer allows beyond agy's own `--print-timeout`.
const OUTER_TIMEOUT_GRACE_S: f64 = 60.0;

/// The tools a critic may use: reading, listing and searching its packet, and `finish`, through which agy returns
/// structured output. Any other tool in the stream voids the review, even when agy refused it.
const READ_ONLY_TOOLS: [&str; 6] = ["read_file", "view_file", "list_dir", "grep_search", "find_by_name", "finish"];

const AGY_ERROR_PREFIX: &str = "AGY_ERROR:";

/// Setup's own check: `agy` must be on `PATH`, and `agy models` must list the routed model. Listing the models needs a
/// signed-in account but calls no model, so it spends no quota and answers even while the quota is spent.
pub fn agy_readiness(runtime: &Runtime, config: &AgyCriticClient) -> Result<ClientReadiness, CritiqueError> {
    if !on_path(runtime, &config.binary) {
        return Ok(ClientReadiness { ready: false, detail: Some(format!("antigravity: {} is not on PATH", config.binary)) });
    }
    let listed = runtime.gateway.process.exec(&config.binary, &["models"], Some(MODELS_TIMEOUT_MS));
    Ok(models_readiness(listed, config))
}

/// What `agy models` says about this machine. It prints one model per line, the id, a tab and a label, after a
/// progress line; signed out, it exits 1 with `Error: Please sign in to view available models. ...`. An error is what
/// kept it from starting.
pub fn models_readiness(listed: Result<ProcessResult, String>, config: &AgyCriticClient) -> ClientReadiness {
    let failed = |reason: String| ClientReadiness { ready: false, detail: Some(format!("antigravity: {} models failed: {reason}", config.binary)) };
    let listed = match listed {
        Ok(listed) => listed,
        Err(reason) => return failed(reason),
    };
    if listed.timed_out {
        return failed(format!("no answer within {}s", MODELS_TIMEOUT_MS / 1000));
    }
    let lines: Vec<String> = format!("{}\n{}", listed.stdout, listed.stderr)
        .split('\n')
        .map(|line| js_trim(line).to_string())
        .filter(|line| !line.is_empty() && line != MODELS_PROGRESS)
        .collect();
    if listed.exit_code != 0 {
        if lines.iter().any(|line| line.to_ascii_lowercase().contains("sign in")) {
            return ClientReadiness { ready: false, detail: Some(SIGN_IN.to_string()) };
        }
        return failed(lines.first().cloned().unwrap_or_else(|| format!("exit {}", listed.exit_code)));
    }
    if lines.iter().any(|line| line.split('\t').next() == Some(config.model.as_str())) {
        return ClientReadiness { ready: true, detail: None };
    }
    ClientReadiness { ready: false, detail: Some(format!("antigravity: {} does not offer {}", config.binary, config.model)) }
}

/// Runs one Gemini critic through `agy` in print mode, with the extras a critic needs: the read-only sandbox profile,
/// the finding schema and the tools its prompt names.
pub fn agy_review(runtime: &Runtime, launch: &CriticLaunch<'_>, config: &AgyCriticClient) -> Result<CriticReport, CritiqueError> {
    let request = ClientRunRequest {
        launch: LaunchRequest {
            model: config.gateway.as_ref().map_or_else(|| config.model.clone(), |gateway| gateway.model.clone()),
            time_limit_s: launch.critic.time_limit_s,
            max_price_usd: None,
            mode: None,
            provider: None,
        },
        binary: config.binary.clone(),
        client: gateway_client(config.gateway.as_ref(), "agy")?,
        prompt: runtime.files.read_text(&launch.prompt_path).map_err(CritiqueError::from_fs)?,
        paths: LaunchPaths {
            export_dir: join(&[&launch.critic_dir, "export"]),
            prompt_path: launch.prompt_path.clone(),
            run_dir: None,
            work_dir: launch.packet_dir.clone(),
            git_writable: None,
        },
        extras: RunExtras { sandbox: Some(SandboxProfile::ReadOnly), schema: Some(launch.schema_path.clone()), tools: Some(tool_names(&launch.toolset)), ..RunExtras::default() },
    };
    let gateway = ClientRun { process: runtime.gateway.process.as_ref(), files: runtime.gateway.files.as_ref(), env: &runtime.gateway.env };
    let result = gateway.launch(&request).map_err(launch_error)?.result;
    let stream = read_stream(&result.stdout);
    let export_path = join(&[&launch.critic_dir, "result.json"]);
    if let Some(found) = &stream.result {
        runtime.files.write_text(&export_path, &format!("{}\n", json::pretty(&Value::Obj(found.clone())))).map_err(CritiqueError::from_fs)?;
    }
    let outcome = if result.timed_out {
        timed_out(launch.critic.time_limit_s + OUTER_TIMEOUT_GRACE_S)
    } else {
        classify_agy_run(result.exit_code, &result.stdout, &stream, &result.stderr)
    };
    // Antigravity is quota, not money: it counts against the daily cap, never the Vibe cap.
    Ok(CriticReport { exit_code: result.exit_code, stdout: result.stdout, stderr: result.stderr, export_path, outcome, charged_usd: 0.0, vibe_cost_usd: None, tokens: None, search_failure: None, tool_calls: Vec::new() })
}

/// What agy's event stream held.
pub struct AgyStream {
    pub tools: Vec<String>,
    pub result: Option<Object>,
}

/// The tools the stream's steps used and its last result event; a line that is not a JSON object ends the reading,
/// and leaves no result.
pub fn read_stream(stdout: &str) -> AgyStream {
    let mut tools = Vec::new();
    let mut result: Option<Object> = None;
    for line in stdout.split('\n').filter(|text| !js_trim(text).is_empty()) {
        let Some(event) = json::parse(line).ok().and_then(|value| value.as_object().cloned()) else {
            return AgyStream { tools, result: None };
        };
        let step = event.get("step_update").and_then(Value::as_object);
        let named = step.and_then(|step| step.get("tool_name")).and_then(Value::as_str);
        if let (Some("tool"), Some(name)) = (step.and_then(|step| step.get("step_type")).and_then(Value::as_str), named) {
            tools.push(name.to_string());
        }
        if event.get("event").and_then(Value::as_str) == Some("result") {
            result = event.get("result").and_then(Value::as_object).cloned();
        }
    }
    AgyStream { tools, result }
}

/// Why agy exited non-zero: the `short_error` of its `AGY_ERROR: {...}` line (a spent quota reads
/// `RESOURCE_EXHAUSTED (code 429): Individual quota reached. ... Resets in <time>.`), else the first line of stderr.
fn exit_reason(stderr: &str) -> Option<String> {
    let lines: Vec<String> = stderr.split('\n').map(|line| js_trim(line).to_string()).filter(|line| !line.is_empty()).collect();
    for line in lines.iter().filter(|text| text.starts_with(AGY_ERROR_PREFIX)) {
        let short = json::parse(&line[AGY_ERROR_PREFIX.len()..]).ok().and_then(|value| value.as_object().and_then(|object| object.get("short_error")).and_then(Value::as_str).map(str::to_string));
        if let Some(short) = short.filter(|short| !short.is_empty()) {
            return Some(short);
        }
    }
    lines.first().cloned()
}

fn timed_out(timeout_s: f64) -> CriticOutcome {
    CriticOutcome::Failed {
        failure: "timeout".to_string(),
        detail: format!("agy did not return within {}s, so the outer timer killed it", json::number_text(timeout_s)),
        cost_usd: None,
    }
}

/// agy reports `SUCCESS` and exits 0 for runs that produced nothing, so the fields decide: an empty response, a
/// denied action, a print timeout or a missing `structured_output` is a failed critic whatever the exit code, and so is
/// any tool beyond reading the packet. Findings come from `structured_output` only, checked against the output
/// contract again.
pub fn classify_agy_run(exit_code: i32, stdout: &str, run: &AgyStream, stderr: &str) -> CriticOutcome {
    let failed = |failure: &str, detail: String| CriticOutcome::Failed { failure: failure.to_string(), detail, cost_usd: None };
    if exit_code != 0 {
        let reason = exit_reason(stderr).map_or_else(String::new, |reason| format!(": {reason}"));
        return failed(&format!("exit-{exit_code}"), format!("agy exited {exit_code}{reason}"));
    }
    if js_trim(stdout).is_empty() {
        return failed("empty-output", "agy exited 0 with empty stdout".to_string());
    }
    let Some(result) = &run.result else {
        return failed("unparseable-output", "stdout is not an agy event stream with a result event".to_string());
    };
    let mut unsafe_tools: Vec<&str> = Vec::new();
    for tool in run.tools.iter().filter(|tool| !READ_ONLY_TOOLS.contains(&tool.as_str())) {
        if !unsafe_tools.contains(&tool.as_str()) {
            unsafe_tools.push(tool);
        }
    }
    if !unsafe_tools.is_empty() {
        return failed("unsafe-tool", format!("the critic used tools beyond reading its packet: {}", unsafe_tools.join(", ")));
    }
    if result.get("finished_reason").and_then(Value::as_str) == Some("print_timeout") {
        return failed("print-timeout", "agy stopped at its print timeout".to_string());
    }
    if let Some(Value::Arr(denied)) = result.get("denied_actions") {
        if !denied.is_empty() {
            let actions: Vec<String> = denied.iter().map(|action| js_string(action.as_object().and_then(|object| object.get("action")))).collect();
            return failed("denied-action", format!("headless agy denied: {}", actions.join(", ")));
        }
    }
    if !result.get("response").and_then(Value::as_str).is_some_and(|response| !js_trim(response).is_empty()) {
        return failed("empty-output", "agy returned an empty response".to_string());
    }
    let Some(structured) = result.get("structured_output") else {
        return failed("missing-structured-output", "the reply did not match the finding schema, so agy returned no structured_output".to_string());
    };
    match read_findings(structured) {
        FindingsParse::Failed { failure, detail } => failed(failure, detail),
        FindingsParse::Ok { findings, no_findings_reason } => CriticOutcome::Ok { findings, no_findings_reason, cost_usd: None },
    }
}
