//! The critic clients: what each makes of a run, how each is launched and when each is ready. The expected words are
//! the node runner's, from `adapters/agy`, `adapters/work` and `adapters/vibe`.

use super::vibe_client::{agy_config, claude_config, codex_config, exit, fake_runtime, vibe_config, FakeFiles, FakeProcess};
use crate::agy_client::{classify_agy_run, models_readiness, read_stream};
use crate::clients::{client_readiness, review_with, ClientReadiness, CriticLaunch, CriticOutcome};
use crate::critic_tools::READ_ONLY_TOOLSET;
use crate::roster::RosterCritic;
use crate::routing::CriticClient;
use crate::vibe_client::{classify_vibe_run, ExpectedConfig, VibeRun};
use model_gateway::json::{compact, parse, Value};
use model_gateway::ports::{Files, ProcessResult};

fn critic(config: CriticClient) -> RosterCritic {
    RosterCritic {
        client: "client-1".to_string(),
        role: "critic".to_string(),
        lenses: vec![],
        max_price_usd: 2.0,
        time_limit_s: 300.0,
        critic_id: "c1".to_string(),
        client_config: config,
        fallbacks: vec![],
    }
}

fn launch(critic: &RosterCritic) -> CriticLaunch<'_> {
    CriticLaunch {
        critic,
        packet_dir: "/run/c1/packet".to_string(),
        prompt_path: "/run/c1/prompt.md".to_string(),
        critic_dir: "/run/c1".to_string(),
        schema_path: "/assets/schemas/finding.schema.json".to_string(),
        toolset: READ_ONLY_TOOLSET,
    }
}

fn failed(outcome: &CriticOutcome) -> (String, String, Option<f64>) {
    match outcome {
        CriticOutcome::Failed { failure, detail, cost_usd } => (failure.clone(), detail.clone(), *cost_usd),
        CriticOutcome::Ok { .. } => panic!("expected a failure"),
    }
}

fn said(code: i32, stdout: &str, stderr: &str) -> ProcessResult {
    ProcessResult { stdout: stdout.to_string(), stderr: stderr.to_string(), exit_code: code, timed_out: false }
}

fn slow(result: ProcessResult) -> ProcessResult {
    ProcessResult { timed_out: true, ..result }
}

fn saved(files: &FakeFiles, path: &str) -> String {
    files.read_text(path).unwrap_or_else(|_| panic!("{path} was not written"))
}

const NONE_FOUND: &str = "```json\n{\"findings\":[],\"no_findings_reason\":\"nothing to find\"}\n```";

#[test]
fn agy_lists_its_models_to_say_whether_it_is_ready() {
    let config = agy_config();
    let ready = |listed| models_readiness(listed, &config);
    let not = |detail: &str| ClientReadiness { ready: false, detail: Some(detail.to_string()) };
    assert_eq!(ready(Ok(said(0, "Fetching available models...\ngemini-3\tGemini 3\n", ""))), ClientReadiness { ready: true, detail: None });
    assert_eq!(ready(Ok(said(0, "gemini-2\tOld\n", ""))), not("antigravity: agy does not offer gemini-3"));
    let signed_out = "Fetching available models...\nError: Please SIGN IN to view available models.\n";
    assert_eq!(ready(Ok(said(1, "", signed_out))), not("antigravity: needs sign-in: run agy and sign in with your Google account"));
    assert_eq!(ready(Ok(said(2, "", "\n  boom \nmore"))), not("antigravity: agy models failed: boom"));
    assert_eq!(ready(Ok(said(3, "Fetching available models...", ""))), not("antigravity: agy models failed: exit 3"));
    assert_eq!(ready(Ok(slow(said(0, "", "")))), not("antigravity: agy models failed: no answer within 20s"));
    assert_eq!(ready(Err("spawn agy ENOENT".to_string())), not("antigravity: agy models failed: spawn agy ENOENT"));
}

#[test]
fn a_client_is_ready_when_its_cli_is_installed_and_signed_in() {
    let files = FakeFiles::default();
    for binary in ["/bin/agy", "/bin/codex", "/bin/claude"] {
        files.put(binary, "");
    }
    let env = [("PATH", "/usr/bin:/bin"), ("HOME", "/home/owner")];
    let process = FakeProcess::answering(said(0, "gemini-3\tGemini\n", ""));
    let runtime = fake_runtime(&files, &process, &env);
    let ok = ClientReadiness { ready: true, detail: None };
    for config in [CriticClient::Agy(agy_config()), CriticClient::Codex(codex_config()), CriticClient::Claude(claude_config())] {
        assert_eq!(client_readiness(&runtime, &config), Ok(ok.clone()));
    }
    let claude = process.commands().last().cloned().expect("the status command");
    assert_eq!((claude.command.as_str(), claude.args.join(" ")), ("claude", "auth status".to_string()));
    assert_eq!(claude.env.get("CLAUDE_CONFIG_DIR").map(String::as_str), Some("/home/owner/.claude"));
    // Nothing installed: every client says where it looked.
    let empty = fake_runtime(&FakeFiles::default(), &process, &env);
    let words = |config: CriticClient| client_readiness(&empty, &config).map(|ready| ready.detail);
    assert_eq!(words(CriticClient::Agy(agy_config())), Ok(Some("antigravity: agy is not on PATH".to_string())));
    assert_eq!(words(CriticClient::Codex(codex_config())), Ok(Some("codex: codex is not on PATH".to_string())));
    assert_eq!(words(CriticClient::Claude(claude_config())), Ok(Some("claude: claude is not on PATH".to_string())));
}

#[test]
fn a_work_client_that_is_not_signed_in_says_why() {
    let files = FakeFiles::default();
    files.put("/bin/codex", "");
    files.put("/bin/claude", "");
    let env = [("PATH", "/bin"), ("HOME", "/home/owner")];
    let detail = |process: &FakeProcess, config: CriticClient| client_readiness(&fake_runtime(&files, process, &env), &config).map(|ready| ready.detail);
    let codex = |reason: &str| Ok(Some(format!("codex: not signed in; run codex login with the work account, then re-run setup ({reason})")));
    let claude = |reason: &str| {
        Ok(Some(format!("claude: not signed in to the work profile; run claude and sign in with the work account, then re-run setup ({reason})")))
    };
    assert_eq!(detail(&FakeProcess::answering(said(1, "", "\nNot logged in\n")), CriticClient::Codex(codex_config())), codex("Not logged in"));
    assert_eq!(detail(&FakeProcess::answering(said(7, "", "")), CriticClient::Claude(claude_config())), claude("exit 7"));
    assert_eq!(detail(&FakeProcess::answering(slow(said(0, "", ""))), CriticClient::Codex(codex_config())), codex("no answer within 20s"));
    assert_eq!(detail(&FakeProcess::refusing("spawn claude EACCES"), CriticClient::Claude(claude_config())), claude("spawn claude EACCES"));
    let no_home = fake_runtime(&files, &FakeProcess::answering(exit(0)), &[("PATH", "/bin")]);
    let error = client_readiness(&no_home, &CriticClient::Claude(claude_config())).err().map(|error| error.message);
    assert_eq!(error, Some("required environment variable is missing: HOME".to_string()));
}

fn stream(lines: &[&str]) -> String {
    lines.join("\n")
}

fn response_of(result: Option<model_gateway::json::Object>) -> Option<String> {
    result.and_then(|result| result.get("response").and_then(Value::as_str).map(str::to_string))
}

#[test]
fn agys_stream_gives_its_tools_and_its_last_result() {
    let found = read_stream(&stream(&[
        "",
        r#"{"step_update":{"step_type":"tool","tool_name":"view_file"}}"#,
        r#"{"step_update":{"step_type":"thought","tool_name":"x"}}"#,
        r#"{"step_update":{"step_type":"tool"}}"#,
        r#"{"event":"result","result":{"response":"one"}}"#,
        r#"{"event":"result","result":{"response":"two"}}"#,
    ]));
    assert_eq!(found.tools, ["view_file"]);
    assert_eq!(response_of(found.result), Some("two".to_string()));
    // A result that is not an object replaces an earlier one.
    assert!(read_stream(&stream(&[r#"{"event":"result","result":{"a":1}}"#, r#"{"event":"result","result":3}"#])).result.is_none());
    // A line that is not an object ends the reading with no result.
    for bad in ["not json", "[1]"] {
        let step = r#"{"step_update":{"step_type":"tool","tool_name":"view_file"}}"#;
        let cut = read_stream(&stream(&[step, r#"{"event":"result","result":{"a":1}}"#, bad]));
        assert!(cut.result.is_none());
        assert_eq!(cut.tools, ["view_file"]);
    }
}

fn classified(exit_code: i32, stdout: &str, stderr: &str) -> CriticOutcome {
    classify_agy_run(exit_code, stdout, &read_stream(stdout), stderr)
}

fn result_line(result: &str) -> String {
    format!(r#"{{"event":"result","result":{result}}}"#)
}

#[test]
fn agys_exit_code_and_its_own_error_come_first() {
    let spent = r#"AGY_ERROR: {"short_error":"RESOURCE_EXHAUSTED (code 429): Individual quota reached."}"#;
    assert_eq!(
        failed(&classified(1, "", &format!("noise\n{spent}\n"))),
        ("exit-1".to_string(), "agy exited 1: RESOURCE_EXHAUSTED (code 429): Individual quota reached.".to_string(), None)
    );
    // An error line that is not JSON, or has no short error, falls back to the first line.
    assert_eq!(failed(&classified(2, "", "AGY_ERROR: nope\nsecond")).1, "agy exited 2: AGY_ERROR: nope");
    assert_eq!(failed(&classified(2, "", "AGY_ERROR: {\"short_error\":\"\"}\nsecond")).1, "agy exited 2: AGY_ERROR: {\"short_error\":\"\"}");
    assert_eq!(failed(&classified(3, "", "\n")).1, "agy exited 3");
}

#[test]
fn agy_reports_success_for_runs_that_produced_nothing() {
    let reply = |result: &str| classified(0, &result_line(result), "");
    assert_eq!(failed(&classified(0, "  \n", "")), ("empty-output".to_string(), "agy exited 0 with empty stdout".to_string(), None));
    assert_eq!(failed(&classified(0, "not json", "")).0, "unparseable-output");
    assert_eq!(failed(&classified(0, r#"{"event":"progress"}"#, "")).1, "stdout is not an agy event stream with a result event");
    let tool = |name: &str| format!(r#"{{"step_update":{{"step_type":"tool","tool_name":"{name}"}}}}"#);
    let unsafe_stream = [tool("run_command"), tool("view_file"), tool("browser"), tool("run_command"), result_line(r#"{"response":"x"}"#)].join("\n");
    assert_eq!(failed(&classified(0, &unsafe_stream, "")).1, "the critic used tools beyond reading its packet: run_command, browser");
    assert_eq!(
        failed(&reply(r#"{"finished_reason":"print_timeout","response":"x"}"#)),
        ("print-timeout".to_string(), "agy stopped at its print timeout".to_string(), None)
    );
    assert_eq!(
        failed(&reply(r#"{"denied_actions":[{"action":"shell"},{"action":3},{},"x"],"response":"x"}"#)),
        ("denied-action".to_string(), "headless agy denied: shell, 3, undefined, undefined".to_string(), None)
    );
    assert_eq!(failed(&reply(r#"{"denied_actions":[],"response":"  "}"#)).1, "agy returned an empty response");
    assert_eq!(failed(&reply(r#"{"response":7}"#)).0, "empty-output");
    assert_eq!(failed(&reply(r#"{"response":"x"}"#)).0, "missing-structured-output");
}

#[test]
fn agys_findings_come_from_its_structured_output_alone() {
    let reply = |structured: &str| classified(0, &result_line(&format!(r#"{{"response":"x","structured_output":{structured}}}"#)), "");
    assert_eq!(failed(&reply("null")).0, "invalid-findings");
    assert_eq!(failed(&reply(r#"{"findings":[]}"#)).0, "missing-no-findings-reason");
    let CriticOutcome::Ok { findings, no_findings_reason, cost_usd } = reply(r#"{"findings":[],"no_findings_reason":"clean"}"#) else {
        panic!("a review")
    };
    assert_eq!((findings.len(), no_findings_reason, cost_usd), (0, Some("clean".to_string()), None));
}

fn prompt_files() -> FakeFiles {
    let files = FakeFiles::default();
    files.put("/run/c1/prompt.md", "the prompt");
    files
}

#[test]
fn an_agy_critic_is_launched_read_only_and_its_result_is_kept() {
    let files = prompt_files();
    let result = result_line(r#"{"response":"x","structured_output":{"findings":[],"no_findings_reason":"clean"}}"#);
    let process = FakeProcess::answering(said(0, &result, "warn"));
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let routed = critic(CriticClient::Agy(agy_config()));
    let report = review_with(&runtime, &launch(&routed)).unwrap_or_else(|error| panic!("{}", error.message));
    assert!(matches!(report.outcome, CriticOutcome::Ok { .. }));
    assert_eq!((report.charged_usd, report.export_path.as_str(), report.stderr.as_str()), (0.0, "/run/c1/result.json", "warn"));
    assert_eq!(report.vibe_cost_usd, None);
    assert_eq!(report.tokens, None);
    assert!(saved(&files, "/run/c1/result.json").contains("\"response\": \"x\""));
    let command = process.commands().remove(0);
    assert_eq!(command.cwd.as_deref(), Some("/run/c1/packet"));
    assert_eq!(command.timeout_ms, Some(360_000));
    for arg in ["gemini-3", "/assets/schemas/finding.schema.json"] {
        assert!(command.args.iter().any(|given| given == arg), "{arg} is not in {:?}", command.args);
    }
}

#[test]
fn an_agy_critic_without_a_result_keeps_no_file_and_a_timeout_is_the_outer_timers() {
    let files = prompt_files();
    let process = FakeProcess::answering(slow(said(0, "not json", "")));
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let routed = critic(CriticClient::Agy(agy_config()));
    let report = review_with(&runtime, &launch(&routed)).unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(failed(&report.outcome), ("timeout".to_string(), "agy did not return within 360s, so the outer timer killed it".to_string(), None));
    assert!(!files.has("/run/c1/result.json"));
}

#[test]
fn a_launch_that_cannot_start_or_read_its_prompt_is_unexpected() {
    let files = prompt_files();
    let routed = critic(CriticClient::Agy(agy_config()));
    let refused = fake_runtime(&files, &FakeProcess::refusing("spawn agy EACCES"), &[]);
    assert_eq!(review_with(&refused, &launch(&routed)).err().map(|error| error.message), Some("spawn agy EACCES".to_string()));
    let bare = fake_runtime(&FakeFiles::default(), &FakeProcess::answering(exit(0)), &[]);
    assert!(review_with(&bare, &launch(&routed)).is_err());
    let mut pinned = agy_config();
    pinned.gateway = Some(crate::routing::GatewayModel { client: "gemini".to_string(), model: "m".to_string() });
    let odd = critic(CriticClient::Agy(pinned));
    assert_eq!(review_with(&refused, &launch(&odd)).err().map(|error| error.code), Some("not-supported-yet".to_string()));
}

const CODEX_ANSWER: &str = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"```json\\n{\\\"findings\\\":[],\\\"no_findings_reason\\\":\\\"clean\\\"}\\n```\"}}\n";

#[test]
fn a_work_critic_is_launched_read_only_and_charges_nothing() {
    let files = prompt_files();
    let process = FakeProcess::answering(said(0, CODEX_ANSWER, ""));
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let routed = critic(CriticClient::Codex(codex_config()));
    let report = review_with(&runtime, &launch(&routed)).unwrap_or_else(|error| panic!("{}", error.message));
    assert_eq!(report.charged_usd, 0.0);
    assert!(report.tokens.is_some());
    let kept = saved(&files, "/run/c1/result.json");
    assert!(kept.contains("\"charged_usd\": null") && kept.contains("\"cached\": 0"), "{kept}");
    let command = process.commands().remove(0);
    assert_eq!(command.cwd.as_deref(), Some("/run/c1/packet"));
    let claude = critic(CriticClient::Claude(claude_config()));
    review_with(&runtime, &launch(&claude)).unwrap_or_else(|error| panic!("{}", error.message));
    let command = process.commands().pop().expect("the claude launch");
    assert_eq!(command.env.get("CLAUDE_CONFIG_DIR").map(String::as_str), Some("/home/owner/.claude"), "{:?}", command.env);
    let no_home = fake_runtime(&files, &process, &[]);
    assert!(review_with(&no_home, &launch(&claude)).is_err());
    // A pinned gateway model names the model, not the client.
    let mut pinned = codex_config();
    pinned.gateway = Some(crate::routing::GatewayModel { client: "other".to_string(), model: "gpt-pinned".to_string() });
    let pinned = critic(CriticClient::Codex(pinned));
    review_with(&runtime, &launch(&pinned)).unwrap_or_else(|error| panic!("{}", error.message));
    assert!(process.commands().pop().is_some_and(|command| command.args.iter().any(|arg| arg == "gpt-pinned")));
}

#[test]
fn a_work_critics_failures_are_worded_with_its_client_name() {
    let routed = critic(CriticClient::Codex(codex_config()));
    let files = prompt_files();
    let outcome = |result: ProcessResult| {
        let runtime = fake_runtime(&files, &FakeProcess::answering(result), &[("HOME", "/home/owner")]);
        failed(&review_with(&runtime, &launch(&routed)).unwrap_or_else(|error| panic!("{}", error.message)).outcome)
    };
    assert_eq!(outcome(slow(said(0, "", ""))).1, "codex did not return within 300s, so the outer timer killed it");
    assert_eq!(outcome(said(2, "", "\nboom\n")).1, "codex exited 2: boom");
    assert_eq!(outcome(said(2, "", "")).1, "codex exited 2");
    assert_eq!(outcome(said(0, "  ", "")).1, "codex exited 0 with empty stdout");
    assert_eq!(outcome(said(0, "{\"type\":\"turn.failed\",\"error\":{\"message\":\"bad things\"}}\n", "")).0, "infrastructure-failure");
    assert_eq!(outcome(said(0, "plain text\n", "")).1, "stdout is not codex's output with a final message");
    let unfenced = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"no block\"}}\n";
    assert_eq!(outcome(said(0, unfenced, "")).0, "no-findings-block");
}

fn journal(text: &str) -> String {
    format!(r#"[{{"type":"message","role":"assistant","content":[{{"text":{}}}]}}]"#, compact(&Value::str(text)))
}

const ISOLATED: &str = r#"{"config":{"enable_mcp":false,"enabled_tools":["read_file"],"include_project_context":false,"outside_workdir":"deny"}"#;

#[test]
fn a_vibe_critics_final_message_is_read_against_the_output_contract() {
    let run = |stdout: String| VibeRun {
        exit_code: 0,
        expected: ExpectedConfig::Off,
        stdout,
        stderr: String::new(),
        exported: parse(&format!("{ISOLATED}}}")).ok(),
        timed_out: false,
        timeout_s: 120.0,
        cost_usd: Some(0.25),
        vibe_cost_usd: Some(0.25),
    };
    let CriticOutcome::Ok { findings, no_findings_reason, cost_usd } = classify_vibe_run(&run(journal(NONE_FOUND))) else { panic!("a review") };
    assert_eq!((findings.len(), no_findings_reason, cost_usd), (0, Some("nothing to find".to_string()), Some(0.25)));
    assert_eq!(failed(&classify_vibe_run(&run(journal("no block")))).0, "no-findings-block");
    assert_eq!(failed(&classify_vibe_run(&run(String::new()))), ("empty-output".to_string(), "vibe exited 0 with empty stdout".to_string(), Some(0.25)));
}

#[test]
fn a_vibe_critic_is_launched_in_its_packet_and_priced_from_its_export() {
    let files = prompt_files();
    files.put("/run/c1/export/export.json", &format!(r#"{ISOLATED},"cost_usd":0.5,"usage":{{"input_tokens":10,"output_tokens":5}}}}"#));
    let process = FakeProcess::answering(said(0, &journal(NONE_FOUND), ""));
    let runtime = fake_runtime(&files, &process, &[("HOME", "/home/owner")]);
    let routed = critic(CriticClient::Vibe(vibe_config()));
    let report = review_with(&runtime, &launch(&routed)).unwrap_or_else(|error| panic!("{}", error.message));
    assert!(matches!(report.outcome, CriticOutcome::Ok { .. }));
    assert_eq!((report.charged_usd, report.vibe_cost_usd, report.export_path.as_str()), (0.5, Some(Some(0.5)), "/run/c1/export/export.json"));
    assert_eq!(report.tokens.map(|tokens| (tokens.input, tokens.output)), Some((10.0, 5.0)));
    let command = process.commands().remove(0);
    assert_eq!(command.cwd.as_deref(), Some("/run/c1/packet"));
}
