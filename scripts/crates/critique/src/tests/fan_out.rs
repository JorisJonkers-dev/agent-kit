//! The fan-out's own choices: which failed critics are retried through which fallback, and the branch the goldens cannot
//! reach, a gateway ledger that will not take a critic's record.

use super::cold_read::Refusing;
use super::manifest::manifest_text;
use super::refute_round::full_run;
use super::scenarios::Sandbox;
use super::tripwire::Status;
use crate::critic::CriticRun;
use crate::fan_out_command::{fallback_retries, run};
use crate::manifest::parse_manifest;
use crate::Runtime;
use model_gateway::adapters::fs::SystemFiles;
use model_gateway::json::{parse, Value};
use model_gateway::ports::{Process, ProcessCommand, ProcessResult};
use std::collections::HashMap;
use std::sync::Mutex;

fn client(kind: &str) -> String {
    match kind {
        "vibe" => r#"{"kind":"vibe","binary":"vibe","agent":"critic-x","model":"m","family":"mistral"}"#.to_string(),
        "agy" => r#"{"kind":"agy","binary":"agy","model":"gemini-3","family":"google"}"#.to_string(),
        "codex" => r#"{"kind":"codex","binary":"codex","model":"gpt-5","family":"openai"}"#.to_string(),
        _ => r#"{"kind":"vibe","binary":"vibe","agent":"critic-y","model":"n","family":"zhipu"}"#.to_string(),
    }
}

/// A manifest whose one critic runs through agy and may be retried through each of `fallbacks`, a kind and a name.
fn manifest_with(distinct: bool, fallbacks: &[(&str, &str)]) -> crate::manifest::RunManifest {
    let list: Vec<String> = fallbacks.iter().map(|(name, kind)| format!(r#"{{"client":"{name}","client_config":{}}}"#, client(kind))).collect();
    let critic = format!(
        r#"[{{"client":"agy-main","role":"critic","lenses":[],"max_price_usd":1,"time_limit_s":60,"critic_id":"c1","client_config":{},"fallbacks":[{}]}}]"#,
        client("agy"),
        list.join(",")
    );
    let text = manifest_text(&[("critics", &critic), ("distinct_families", if distinct { "true" } else { "false" })]);
    parse_manifest(&text).expect("a manifest")
}

fn run_of(status: &str, family: &str) -> CriticRun {
    CriticRun { status: status.to_string(), family: family.to_string(), ..full_run() }
}

fn names(retries: &[(crate::roster::RosterCritic, String)]) -> Vec<(String, String)> {
    retries.iter().map(|(critic, of)| (critic.critic_id.clone(), of.clone())).collect()
}

#[test]
fn a_failed_critic_is_retried_through_the_first_fallback_that_may_run() {
    let manifest = manifest_with(true, &[("agy-b", "agy"), ("vibe-x", "vibe"), ("codex-w", "codex"), ("vibe-y", "other")]);
    let failed = [&run_of("failed", "google")];
    // Nothing blocks the first fallback, and the retry has no fallbacks of its own.
    let retries = fallback_retries(&manifest, &failed, (false, false));
    assert_eq!(names(&retries), [("c1-retry-agy-b".to_string(), "c1".to_string())]);
    assert!(retries[0].0.fallbacks.is_empty());
    // agy at its quota is passed over, then Vibe at its plan limit, then Codex runs.
    assert_eq!(names(&fallback_retries(&manifest, &failed, (true, false))), [("c1-retry-vibe-x".to_string(), "c1".to_string())]);
    assert_eq!(names(&fallback_retries(&manifest, &failed, (true, true))), [("c1-retry-codex-w".to_string(), "c1".to_string())]);
    // A critic that violated read-only, was never launched or reviewed is not retried.
    for status in ["violated-readonly", "budget-skip", "ok"] {
        assert!(fallback_retries(&manifest, &[&run_of(status, "google")], (false, false)).is_empty());
    }
}

#[test]
fn a_retry_brings_no_family_that_already_reviewed() {
    let manifest = manifest_with(true, &[("vibe-x", "vibe"), ("vibe-y", "other")]);
    let (reviewer, failed) = (run_of("ok", "mistral"), run_of("failed", "google"));
    assert_eq!(names(&fallback_retries(&manifest, &[&failed, &reviewer], (false, false))), [("c1-retry-vibe-y".to_string(), "c1".to_string())]);
    // The stage that does not need distinct families takes the first fallback whatever reviewed.
    let relaxed = manifest_with(false, &[("vibe-x", "vibe"), ("vibe-y", "other")]);
    assert_eq!(names(&fallback_retries(&relaxed, &[&failed, &reviewer], (false, false))), [("c1-retry-vibe-x".to_string(), "c1".to_string())]);
    // With every fallback's family taken there is no retry.
    let only = manifest_with(true, &[("vibe-x", "vibe")]);
    assert!(fallback_retries(&only, &[&failed, &reviewer], (false, false)).is_empty());
}

/// An `agy` that answers every launch with a review that finds nothing.
struct Reviewing(Mutex<Vec<ProcessCommand>>);

impl Process for Reviewing {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Err("no keyring".to_string())
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.0.lock().map_err(|error| error.to_string())?.push(command.clone());
        let result = r#"{"event":"result","result":{"response":"x","structured_output":{"findings":[],"no_findings_reason":"clean"}}}"#;
        Ok(ProcessResult { stdout: format!("{result}\n"), stderr: String::new(), exit_code: 0, timed_out: false })
    }
}

#[test]
fn a_gateway_ledger_that_will_not_take_a_record_is_a_warning_in_the_result() {
    ledger_refusing_fan_out(&[]);
}

#[test]
fn a_manifest_with_a_ticket_check_fans_out_the_same_way() {
    ledger_refusing_fan_out(&[("ticket_check", TICKET_CHECK)]);
}

/// The ticket check a tickets-stage manifest carries.
pub const TICKET_CHECK: &str = r#"{"cycles":[],"dangling_blockers":[],"status":"passed","ticket_ids":["T1"],"uncited_stories":[]}"#;

/// One fan-out over a manifest with `extra` fields, through a gateway ledger that refuses every record.
fn ledger_refusing_fan_out(extra: &[(&str, &str)]) {
    let sandbox = Sandbox::new();
    let root = sandbox.text();
    let write = |path: &str, text: &str| {
        let target = sandbox.path.join(path);
        assert!(std::fs::create_dir_all(target.parent().unwrap_or(&sandbox.path)).is_ok());
        assert!(std::fs::write(target, text).is_ok());
    };
    let critic = format!(
        r#"[{{"client":"agy-main","role":"spec-redteam","lenses":[],"max_price_usd":1,"time_limit_s":60,"critic_id":"c1","client_config":{},"fallbacks":[]}}]"#,
        client("agy")
    );
    let (packet_dir, repo_root, run_dir) = (format!("\"{root}/run/packet\""), format!("\"{root}/repo\""), format!("\"{root}/run\""));
    let mut fields = vec![("critics", critic.as_str()), ("packet_dir", packet_dir.as_str()), ("repo_root", repo_root.as_str()), ("run_dir", run_dir.as_str())];
    fields.extend_from_slice(extra);
    let text = manifest_text(&fields);
    write("run/manifest.json", &text);
    let mut runtime: Runtime = super::cli::runtime("");
    runtime.env = HashMap::from([("HOME".to_string(), format!("{root}/home")), ("PATH".to_string(), format!("{root}/bin")), ("XDG_STATE_HOME".to_string(), format!("{root}/state"))]);
    runtime.files = Box::new(SystemFiles { cwd: root.clone() });
    runtime.git = Box::new(Status(Mutex::new(Ok(Vec::new()))));
    runtime.gateway.env = runtime.env.clone();
    runtime.gateway.files = Box::new(Refusing(SystemFiles { cwd: root.clone() }));
    runtime.gateway.process = Box::new(Reviewing(Mutex::new(Vec::new())));
    runtime.assets_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../skills/workstation/critique").to_string_lossy().into_owned();
    let input = parse(&format!(r#"{{"run_dir":"{root}/run"}}"#)).unwrap_or(Value::Null);
    let result = run(&input, &runtime).unwrap_or_else(|error| panic!("{}", error.message));
    let warnings = match result.as_object().and_then(|result| result.get("ledger_warnings")) {
        Some(Value::Arr(items)) => items.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert_eq!(warnings.len(), 1, "{}", model_gateway::json::pretty(&result));
    assert!(warnings[0].contains("EACCES"), "{warnings:?}");
    assert!(std::fs::read_to_string(format!("{root}/run/findings.json")).is_ok_and(|text| text.contains("\"ledger_warnings\"")));
}
