//! The work critic clients: Codex on the nedap account and the work Claude agent, which only a `nedap` repository's work
//! roster names (ADR-0008, ADR-0011). Both launch through the model gateway's launch path, read-only, in the critic's own
//! copy of the packet, and read their sign-in from the CLI's own status command, which calls no model and so spends
//! nothing. The node runner's `adapters/work/index.ts`.

use crate::clients::{ClientReadiness, CriticLaunch, CriticOutcome, CriticReport, CriticTokens};
use crate::error::CritiqueError;
use crate::findings::{parse_findings, FindingsParse};
use crate::paths::join;
use crate::routing::{ClaudeCriticClient, CodexCriticClient, GatewayModel};
use crate::runtime::Runtime;
use crate::vibe_client::{gateway_client, launch_error, on_path};
use model_gateway::client_run::{ClientRun, ClientRunOutcome, ClientRunRequest};
use model_gateway::extras::RunExtras;
use model_gateway::json::{self, Value};
use model_gateway::launchers::{LaunchPaths, LaunchRequest};
use model_gateway::ports::{ProcessCommand, ProcessResult};
use model_gateway::run::RunMode;
use model_gateway::text::js_trim;
use std::collections::BTreeMap;

/// The longest a status command may take; signed in it answers at once.
const STATUS_TIMEOUT_MS: u64 = 20_000;

/// The first non-blank line of what a command printed.
fn first_line(result: &ProcessResult) -> Option<String> {
    format!("{}\n{}", result.stdout, result.stderr).split('\n').map(|line| js_trim(line).to_string()).find(|line| !line.is_empty())
}

/// Why a status command could not say, as one clause.
fn unreadable(result: &Result<ProcessResult, String>) -> String {
    match result {
        Err(reason) => reason.clone(),
        Ok(result) if result.timed_out => format!("no answer within {}s", STATUS_TIMEOUT_MS / 1000),
        Ok(result) => first_line(result).unwrap_or_else(|| format!("exit {}", result.exit_code)),
    }
}

/// Whether a status command ran, in time, and exited 0.
fn signed_in(result: &Result<ProcessResult, String>) -> bool {
    matches!(result, Ok(result) if !result.timed_out && result.exit_code == 0)
}

fn missing(client: &str, binary: &str) -> ClientReadiness {
    ClientReadiness { ready: false, detail: Some(format!("{client}: {binary} is not on PATH")) }
}

/// Signed in is `codex login status`, which exits 0 only while it holds the work account's login. A Codex critic needs
/// no file of its own, so this is its readiness too.
pub fn codex_sign_in(runtime: &Runtime, config: &CodexCriticClient) -> Result<ClientReadiness, CritiqueError> {
    if !on_path(runtime, &config.binary) {
        return Ok(missing("codex", &config.binary));
    }
    let status = runtime.gateway.process.exec(&config.binary, &["login", "status"], Some(STATUS_TIMEOUT_MS));
    if signed_in(&status) {
        return Ok(ClientReadiness { ready: true, detail: None });
    }
    Ok(ClientReadiness { ready: false, detail: Some(format!("codex: not signed in; run codex login with the work account, then re-run setup ({})", unreadable(&status))) })
}

/// The work profile's directory, set explicitly so an ambient `CLAUDE_CONFIG_DIR` cannot pick another.
fn profile(runtime: &Runtime) -> Result<BTreeMap<String, String>, CritiqueError> {
    match runtime.env.get("HOME") {
        Some(home) => Ok([("CLAUDE_CONFIG_DIR".to_string(), join(&[home, ".claude"]))].into_iter().collect()),
        None => Err(CritiqueError::unexpected("required environment variable is missing: HOME")),
    }
}

/// Signed in is `claude auth status` for the work configuration directory, `~/.claude`.
pub fn claude_sign_in(runtime: &Runtime, config: &ClaudeCriticClient) -> Result<ClientReadiness, CritiqueError> {
    if !on_path(runtime, &config.binary) {
        return Ok(missing("claude", &config.binary));
    }
    let command = ProcessCommand {
        command: config.binary.clone(),
        args: vec!["auth".to_string(), "status".to_string()],
        env: profile(runtime)?,
        timeout_ms: Some(STATUS_TIMEOUT_MS),
        ..ProcessCommand::default()
    };
    let status = runtime.gateway.process.run(&command);
    if signed_in(&status) {
        return Ok(ClientReadiness { ready: true, detail: None });
    }
    Ok(ClientReadiness {
        ready: false,
        detail: Some(format!("claude: not signed in to the work profile; run claude and sign in with the work account, then re-run setup ({})", unreadable(&status))),
    })
}

/// What a work critic's run amounts to. The exit code and the client's own failure come first, then the final message
/// must hold the findings block. The work critics are paid by the work accounts, so a run charges nothing against the
/// flow's Vibe cap.
fn classify(client: &str, launch: &CriticLaunch<'_>, run: &ClientRunOutcome) -> CriticOutcome {
    let (output, result) = (&run.output, &run.result);
    let cost = output.charged;
    let failed = |failure: &str, detail: String| CriticOutcome::Failed { failure: failure.to_string(), detail, cost_usd: cost };
    if result.timed_out {
        return failed("timeout", format!("{client} did not return within {}s, so the outer timer killed it", json::number_text(launch.critic.time_limit_s)));
    }
    if result.exit_code != 0 {
        let said = first_line(result).map_or_else(String::new, |said| format!(": {said}"));
        return failed(&format!("exit-{}", result.exit_code), format!("{client} exited {}{said}", result.exit_code));
    }
    if js_trim(&result.stdout).is_empty() {
        return failed("empty-output", format!("{client} exited 0 with empty stdout"));
    }
    if output.failure.is_some() {
        let said = output.error.as_ref().map_or_else(String::new, |error| format!(": {}", error.text));
        return failed("infrastructure-failure", format!("{client} reported a failed run{said}"));
    }
    let Some(answer) = &output.answer else {
        return failed("unparseable-output", format!("stdout is not {client}'s output with a final message"));
    };
    match parse_findings(answer) {
        FindingsParse::Failed { failure, detail } => failed(failure, detail),
        FindingsParse::Ok { findings, no_findings_reason } => CriticOutcome::Ok { findings, no_findings_reason, cost_usd: cost },
    }
}

/// The report of one launch: the process's own output, what the run used, and the outcome.
fn report(runtime: &Runtime, client: &str, launch: &CriticLaunch<'_>, run: &ClientRunOutcome) -> Result<CriticReport, CritiqueError> {
    let export_path = join(&[&launch.critic_dir, "result.json"]);
    let output = &run.output;
    let number = |value: f64| Value::Num(value);
    let saved = json::object(vec![
        ("answer", output.answer.as_ref().map_or(Value::Null, |answer| Value::str(answer))),
        ("charged_usd", output.charged.map_or(Value::Null, number)),
        ("tokens", json::object(vec![("cached", number(output.tokens.cached)), ("input", number(output.tokens.input)), ("output", number(output.tokens.output))])),
    ]);
    runtime.files.write_text(&export_path, &format!("{}\n", json::pretty(&saved))).map_err(CritiqueError::from_fs)?;
    Ok(CriticReport {
        exit_code: run.result.exit_code,
        stdout: run.result.stdout.clone(),
        stderr: run.result.stderr.clone(),
        export_path,
        outcome: classify(client, launch, run),
        charged_usd: 0.0,
        vibe_cost_usd: None,
        tokens: Some(CriticTokens { cached: output.tokens.cached, input: output.tokens.input, output: output.tokens.output }),
        search_failure: None,
        tool_calls: Vec::new(),
    })
}

fn launch(
    runtime: &Runtime,
    client: &str,
    launch: &CriticLaunch<'_>,
    binary: &str,
    gateway: Option<&GatewayModel>,
    model: &str,
    extras: RunExtras,
) -> Result<ClientRunOutcome, CritiqueError> {
    let request = ClientRunRequest {
        launch: LaunchRequest {
            model: gateway.map_or_else(|| model.to_string(), |gateway| gateway.model.clone()),
            time_limit_s: launch.critic.time_limit_s,
            max_price_usd: None,
            mode: Some(RunMode::ReadOnly),
            provider: None,
        },
        binary: binary.to_string(),
        // The work clients run on their own gateway client, whatever the config pins as the model's alias.
        client: gateway_client(None, client)?,
        prompt: runtime.files.read_text(&launch.prompt_path).map_err(CritiqueError::from_fs)?,
        paths: LaunchPaths {
            export_dir: join(&[&launch.critic_dir, "export"]),
            prompt_path: launch.prompt_path.clone(),
            run_dir: None,
            work_dir: launch.packet_dir.clone(),
            git_writable: None,
        },
        extras,
    };
    let gateway = ClientRun { process: runtime.gateway.process.as_ref(), files: runtime.gateway.files.as_ref(), env: &runtime.gateway.env };
    gateway.launch(&request).map_err(launch_error)
}

/// `codex exec` in its read-only sandbox, in the critic's own copy of the packet.
pub fn codex_review(runtime: &Runtime, critic: &CriticLaunch<'_>, config: &CodexCriticClient) -> Result<CriticReport, CritiqueError> {
    let run = launch(runtime, "codex", critic, &config.binary, config.gateway.as_ref(), &config.model, RunExtras::default())?;
    report(runtime, "codex", critic, &run)
}

/// Claude Code in plan mode with the read tools alone, in the critic's own copy of the packet, always in the work
/// configuration directory.
pub fn claude_review(runtime: &Runtime, critic: &CriticLaunch<'_>, config: &ClaudeCriticClient) -> Result<CriticReport, CritiqueError> {
    let extras = RunExtras { env: Some(profile(runtime)?), ..RunExtras::default() };
    let run = launch(runtime, "claude", critic, &config.binary, config.gateway.as_ref(), &config.model, extras)?;
    report(runtime, "claude", critic, &run)
}
