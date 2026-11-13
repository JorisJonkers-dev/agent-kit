//! The refute round's own pieces: a refuter's run as the files print it, how a reply is judged, the accessors the round
//! reads a client by, and the one branch the goldens cannot reach: a gateway ledger that will not take a refuter's record.

use super::cold_read::Refusing;
use super::scenarios::Sandbox;
use super::tripwire::Status;
use super::vibe_client::{agy_config, claude_config, codex_config, vibe_config};
use crate::clients::CriticTokens;
use crate::critic::CriticRun;
use crate::evidence::index_packet;
use crate::findings::{CriticFinding, Evidence, FindingBody, RunFinding};
use crate::flow_budget::PromptHashes;
use crate::refute::RefuteRecord;
use crate::refute_command::run;
use crate::refute_round::{judge_reply, record_json, run_json, JudgeContext, RefuterReply};
use crate::routing::{CriticClient, GatewayModel};
use crate::Runtime;
use model_gateway::adapters::fs::SystemFiles;
use model_gateway::json::{compact, parse, Value};
use model_gateway::ports::{Process, ProcessCommand, ProcessResult};
use std::collections::HashMap;
use std::sync::Mutex;

fn full_run() -> CriticRun {
    CriticRun {
        critic_id: "r-f1-agy".to_string(),
        client: "agy".to_string(),
        family: "google".to_string(),
        model: "gemini".to_string(),
        role: "refuter".to_string(),
        lenses: vec!["lens-a".to_string()],
        retry_of: Some("c1".to_string()),
        status: "failed".to_string(),
        failure: Some("timeout".to_string()),
        detail: Some("too slow".to_string()),
        no_findings_reason: Some("nothing".to_string()),
        exit_code: Some(1.0),
        cost_usd: Some(0.5),
        vibe_cost_usd: Some(None),
        tokens: Some(CriticTokens { cached: 1.0, input: 2.0, output: 3.0 }),
        charged_usd: 0.25,
        duration_ms: 12.5,
        finding_count: 0.0,
        prompt_hashes: PromptHashes { contract: "c".to_string(), preamble: "p".to_string(), role: "r".to_string(), schema: "s".to_string() },
        lens_hashes: vec![("lens-a".to_string(), "h".to_string())],
        prompt_path: "/p".to_string(),
        stdout_path: Some("/o".to_string()),
        export_path: Some("/e".to_string()),
        tools: Some(vec!["view_file".to_string()]),
        search_failure: Some("why".to_string()),
    }
}

#[test]
fn a_run_prints_its_fields_in_the_order_the_node_runner_builds_them() {
    let expected = concat!(
        r#"{"client":"agy","critic_id":"r-f1-agy","family":"google","lenses":["lens-a"],"model":"gemini","role":"refuter","#,
        r#""lens_hashes":{"lens-a":"h"},"prompt_hashes":{"contract":"c","preamble":"p","role":"r","schema":"s"},"prompt_path":"/p","#,
        r#""retry_of":"c1","charged_usd":0.25,"cost_usd":0.5,"duration_ms":12.5,"exit_code":1,"export_path":"/e","finding_count":0,"#,
        r#""stdout_path":"/o","vibe_cost_usd":null,"tokens":{"cached":1,"input":2,"output":3},"tools":["view_file"],"search_failure":"why","#,
        r#""detail":"too slow","failure":"timeout","status":"failed","no_findings_reason":"nothing"}"#
    );
    assert_eq!(compact(&run_json(&full_run())), expected);
}

#[test]
fn a_run_that_reports_nothing_prints_only_what_it_has() {
    let bare = CriticRun {
        retry_of: None,
        failure: None,
        detail: None,
        no_findings_reason: None,
        exit_code: None,
        cost_usd: None,
        vibe_cost_usd: None,
        tokens: None,
        stdout_path: None,
        export_path: None,
        tools: None,
        search_failure: None,
        lenses: vec![],
        lens_hashes: vec![],
        status: "budget-skip".to_string(),
        ..full_run()
    };
    let printed = compact(&run_json(&bare));
    assert!(printed.starts_with(r#"{"client":"agy","critic_id":"r-f1-agy","family":"google","lenses":[],"model":"gemini","role":"refuter","lens_hashes":{},"#), "{printed}");
    assert!(printed.ends_with(r#""charged_usd":0.25,"cost_usd":null,"duration_ms":12.5,"exit_code":null,"finding_count":0,"status":"budget-skip"}"#), "{printed}");
}

fn body(check: Option<&str>, severity: &str) -> FindingBody {
    FindingBody {
        severity: severity.to_string(),
        category: "bug".to_string(),
        location: "artifact/spec.md".to_string(),
        claim: "claim".to_string(),
        evidence: vec![Evidence { kind: "quote".to_string(), value: "The retry budget is unbounded.".to_string(), file: Some("artifact/spec.md".to_string()) }],
        suggestion: Some("fix".to_string()),
        check: check.map(str::to_string),
    }
}

fn subject() -> RunFinding {
    RunFinding { id: "f1".to_string(), body: body(None, "critical"), critic_finding_id: "x".to_string(), raised_by: vec!["c1".to_string()], pre_verdict: None, declared_severity: None }
}

fn judged(run: CriticRun, findings: Vec<CriticFinding>) -> RefuteRecord {
    let packet = index_packet(&[], &[]);
    let declared = ["refute".to_string(), "downgrade".to_string()];
    judge_reply(&subject(), &RefuterReply { run, findings }, &JudgeContext { declared: &declared, stage: "spec", packet: &packet, role: None })
}

#[test]
fn a_reply_is_judged_by_the_refuters_run_and_its_answer() {
    let mut failed = full_run();
    failed.detail = None;
    assert_eq!(judged(failed.clone(), vec![]).detail.as_deref(), Some("failed"));
    failed.detail = Some("exit 3".to_string());
    let record = judged(failed, vec![]);
    assert_eq!((record.outcome.as_str(), record.reason_code.as_deref(), record.detail.as_deref()), ("stands", Some("no-review"), Some("exit 3")));
    let mut ok = full_run();
    ok.status = "ok".to_string();
    ok.no_findings_reason = None;
    let none = judged(ok.clone(), vec![]);
    assert_eq!((none.outcome.as_str(), none.answer.as_deref(), none.detail), ("no-objection", Some("no-objection"), None));
    let objection = |check: Option<&str>, severity: &str| CriticFinding { id: "o1".to_string(), body: body(check, severity) };
    let nameless = judged(ok.clone(), vec![objection(None, "major")]);
    assert_eq!(nameless.detail.as_deref(), Some("names the check (none); a refuter answers refute or downgrade"));
    assert_eq!(judged(ok.clone(), vec![objection(Some("refute"), "major"), objection(Some("refute"), "major")]).detail.as_deref(), Some("answered with 2 entries; a refuter gives one"));
    let leaving = judged(ok.clone(), vec![objection(Some("downgrade"), "critical")]);
    assert_eq!((leaving.answer.as_deref(), leaving.reason_code.as_deref()), (Some("downgrade"), Some("invalid-answer")));
    // The evidence is the packet's to check: this one holds none, so the verdict stands.
    let unproven = judged(ok, vec![objection(Some("refute"), "major")]);
    assert_eq!((unproven.outcome.as_str(), unproven.reason_code.as_deref()), ("stands", Some("fabricated-evidence")));
    assert_eq!(compact(&record_json(&unproven)).matches("\"reply\"").count(), 1);
}

#[test]
fn a_client_is_known_by_its_family_model_and_pinned_gateway_model() {
    let pinned = GatewayModel { client: "vibe".to_string(), model: "pinned".to_string() };
    let mut vibe = vibe_config();
    vibe.gateway = Some(pinned.clone());
    let mut claude = claude_config();
    claude.gateway = Some(pinned.clone());
    let all = [CriticClient::Vibe(vibe), CriticClient::Agy(agy_config()), CriticClient::Codex(codex_config()), CriticClient::Claude(claude)];
    let seen: Vec<(&str, &str, bool)> = all.iter().map(|client| (client.family(), client.model(), client.gateway().is_some())).collect();
    assert_eq!(
        seen,
        [("mistral", "mistral-medium-3", true), ("google", "gemini-3", false), ("openai", "gpt-5", false), ("anthropic", "opus", true)]
    );
}

/// An `agy` that lists its model and answers every launch with an objection-free review.
struct Reviewing(Mutex<Vec<ProcessCommand>>);

impl Process for Reviewing {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Ok(ProcessResult { stdout: "gemini-3\tGemini\n".to_string(), stderr: String::new(), exit_code: 0, timed_out: false })
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.0.lock().map_err(|error| error.to_string())?.push(command.clone());
        let result = r#"{"event":"result","result":{"response":"x","structured_output":{"findings":[],"no_findings_reason":"holds"}}}"#;
        Ok(ProcessResult { stdout: format!("{result}\n"), stderr: String::new(), exit_code: 0, timed_out: false })
    }
}

#[test]
fn a_gateway_ledger_that_will_not_take_a_record_is_a_warning_in_the_result() {
    let sandbox = Sandbox::new();
    let root = sandbox.text();
    let write = |path: &str, text: &str| {
        let target = sandbox.path.join(path);
        assert!(std::fs::create_dir_all(target.parent().unwrap_or(&sandbox.path)).is_ok());
        assert!(std::fs::write(target, text).is_ok());
    };
    write("bin/agy", "");
    write("repo/README.md", "# r\n");
    let manifest = format!(
        r#"{{"allowed_families":["google"],"artifact":{{"labels":[],"reference":"o/r#1","tracker":"github"}},"created_at":"2026-10-30T12:00:00.000Z","critics":[],"depth":"base","depth_choice":{{"chosen_by":"argument","measures":{{}},"signals":[]}},"distinct_families":true,"dropped":[],"flow":"f","limits":{{"antigravity_quota":{{"daily_cap":20,"reserve_threshold":10,"reserved_stages":["spec"]}},"search":{{"max_bytes":8192,"max_matches":50}}}},"orchestrator_family":"anthropic","packet_dir":"{root}/run/packet","packet_files":[],"repo_root":"{root}/repo","repository":"o/r","run_dir":"{root}/run","run_id":"r1","stage":"spec","version":1}}"#
    );
    let critic = r#"{"charged_usd":0,"client":"c","cost_usd":null,"critic_id":"c1","duration_ms":0,"exit_code":0,"family":"mistral","finding_count":1,"lens_hashes":{},"lenses":[],"model":"m","prompt_hashes":{"contract":"c","preamble":"p","role":"r","schema":"s"},"prompt_path":"/p","role":"spec-redteam","status":"ok"}"#;
    let finding = r#"{"category":"bug","claim":"c","critic_finding_id":"x","evidence":[],"id":"f1","location":"l","raised_by":["c1"],"severity":"critical"}"#;
    write("run/manifest.json", &manifest);
    write(
        "run/findings.json",
        &format!(r#"{{"critics":[{critic}],"degraded":false,"findings":[{finding}],"findings_path":"{root}/run/findings.json","run_id":"r1","stage":"spec","status":"reviewed"}}"#),
    );
    write(
        "home/.config/agent-kit/critique-routing.json",
        r#"{"clients":{"agy-test":{"kind":"agy","binary":"agy","model":"gemini-3","family":"google"}},"fallbacks":{},"stages":{"spec":{"base":[{"client":"agy-test","role":"spec-redteam","lenses":[],"max_price_usd":1,"time_limit_s":60}],"deep":[]}}}"#,
    );
    let mut runtime: Runtime = super::cli::runtime("");
    runtime.env = HashMap::from([("HOME".to_string(), format!("{root}/home")), ("PATH".to_string(), format!("{root}/bin")), ("XDG_STATE_HOME".to_string(), format!("{root}/state"))]);
    runtime.files = Box::new(SystemFiles { cwd: root.clone() });
    runtime.git = Box::new(Status(Mutex::new(Ok(Vec::new()))));
    runtime.gateway.env = runtime.env.clone();
    runtime.gateway.files = Box::new(Refusing(SystemFiles { cwd: root.clone() }));
    runtime.gateway.process = Box::new(Reviewing(Mutex::new(Vec::new())));
    runtime.assets_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../skills/workstation/critique").to_string_lossy().into_owned();
    let input = parse(&format!(r#"{{"run_dir":"{root}/run","verdicts":[{{"finding_id":"f1","verdict":"accepted"}}]}}"#)).unwrap_or(Value::Null);
    let result = run(&input, &runtime).unwrap_or_else(|error| panic!("{}", error.message));
    let warnings = match result.as_object().and_then(|result| result.get("ledger_warnings")) {
        Some(Value::Arr(items)) => items.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert_eq!(warnings.len(), 1, "{}", model_gateway::json::pretty(&result));
    assert!(warnings[0].contains("EACCES"), "{warnings:?}");
    assert!(std::fs::read_to_string(format!("{root}/run/refute.json")).is_ok_and(|text| text.contains("\"no-objection\"")));
}
