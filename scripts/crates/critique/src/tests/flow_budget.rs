//! The flow budget's invocation logging: both ledger lines byte for byte, and
//! the gateway ledger that will not take the record.

use crate::flow_budget::{critique_ledger_record, CriticTokens, CritiqueLedgerContext, FlowBudget, GatewayClient, InvocationContext, LedgerClient, LedgerRun, PromptHashes};
use crate::Runtime;
use model_gateway::ports::Clock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const AT_MS: i64 = 1_793_707_200_000;
const AT: &str = "2026-11-03T12:00:00.000Z";

struct Frozen;

impl Clock for Frozen {
    fn now_ms(&self) -> i64 {
        AT_MS
    }

    fn monotonic_ms(&self) -> f64 {
        0.0
    }
}

fn temp_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("critique-flow-budget-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

fn env_for(home: &Path) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    env.insert("XDG_STATE_HOME".to_string(), home.join("state").to_string_lossy().into_owned());
    env
}

fn runtime_with(env: HashMap<String, String>) -> Runtime {
    let mut gateway = model_gateway::default_runtime();
    gateway.env = env.clone();
    gateway.clock = Box::new(Frozen);
    Runtime {
        gateway,
        clock: Box::new(Frozen),
        files: Box::new(model_gateway::adapters::fs::SystemFiles { cwd: "/".to_string() }),
        dirs: Box::new(crate::dirs::SystemDirs),
        git: Box::new(crate::git::SystemGit { env: env.clone() }),
        env,
        assets_root: "/assets".to_string(),
        stdin: Box::new(String::new),
    }
}

fn manifest() -> InvocationContext {
    InvocationContext {
        flow: "feature".to_string(),
        repository: "acme/widgets".to_string(),
        round: 2.0,
        run_id: "run-1".to_string(),
        stage: "feature-review".to_string(),
    }
}

fn context() -> CritiqueLedgerContext {
    CritiqueLedgerContext {
        flow: "feature".to_string(),
        repository: "acme/widgets".to_string(),
        round: 2.0,
        run_id: "run-1".to_string(),
        spec: Some("specs/0001.md".to_string()),
        stage: "feature-review".to_string(),
    }
}

fn client() -> LedgerClient {
    LedgerClient { kind: "vibe".to_string(), gateway: Some(GatewayClient { client: "vibe-cli".to_string(), model: "vibe-large".to_string() }) }
}

fn full_run() -> LedgerRun {
    LedgerRun {
        charged_usd: 0.5,
        client: "vibe".to_string(),
        cost_usd: Some(0.4),
        critic_id: "security".to_string(),
        exit_code: Some(0.0),
        family: "vibe".to_string(),
        model: "vibe-small".to_string(),
        status: "ok".to_string(),
        duration_ms: Some(12_500.0),
        failure: None,
        finding_count: Some(3.0),
        lens_hashes: Some(vec![("security".to_string(), "abc".to_string())]),
        lenses: Some(vec!["security".to_string()]),
        prompt_hashes: Some(PromptHashes {
            preamble: "p".to_string(),
            role: "r".to_string(),
            contract: "c".to_string(),
            schema: "s".to_string(),
        }),
        retry_of: Some("critic-0".to_string()),
        role: Some("reviewer".to_string()),
        tokens: Some(CriticTokens { cached: 10.0, input: 100.0, output: 20.0 }),
        vibe_cost_usd: Some(Some(0.25)),
    }
}

fn bare_run() -> LedgerRun {
    LedgerRun {
        charged_usd: 0.0,
        client: "agy".to_string(),
        cost_usd: None,
        critic_id: "triage".to_string(),
        exit_code: None,
        family: "agy".to_string(),
        model: "gemini".to_string(),
        status: "failed".to_string(),
        duration_ms: None,
        failure: None,
        finding_count: None,
        lens_hashes: None,
        lenses: None,
        prompt_hashes: None,
        retry_of: None,
        role: None,
        tokens: None,
        vibe_cost_usd: None,
    }
}

fn bare_manifest() -> InvocationContext {
    InvocationContext {
        flow: "feature".to_string(),
        repository: "acme/widgets".to_string(),
        round: 1.0,
        run_id: "run-2".to_string(),
        stage: "triage".to_string(),
    }
}

fn bare_client() -> LedgerClient {
    LedgerClient { kind: "agy".to_string(), gateway: None }
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).expect("ledger")
}

#[test]
fn logs_both_ledgers_for_an_ok_run() {
    let home = temp_dir();
    let runtime = runtime_with(env_for(&home));
    let budget = FlowBudget::new(&runtime);
    let message = budget.log_invocation(&manifest(), &full_run(), &client(), Some("specs/0001.md")).expect("logged");
    assert_eq!(message, None);
    let own = crate::locations::ledger_path(&runtime.env).expect("own ledger");
    assert_eq!(
        read(&own),
        format!(
            "{}\n",
            concat!(
                r#"{"charged_usd":0.5,"client":"vibe","critic_id":"security","family":"vibe","flow":"feature","kind":"vibe","#,
                r#""model":"vibe-small","recorded_at":"2026-11-03T12:00:00.000Z","repository":"acme/widgets","round":2,"#,
                r#""run_id":"run-1","stage":"feature-review","status":"ok","type":"invocation","vibe_cost_usd":0.25}"#
            )
        )
    );
    let gateway = model_gateway::locations::ledger_path(&runtime.gateway.env).expect("gateway ledger");
    assert_eq!(
        read(&gateway),
        format!(
            "{}\n",
            concat!(
                r#"{"at":"2026-11-03T12:00:00.000Z","charged_usd":0.5,"client":"vibe-cli","cost_usd":0.5,"#,
                r#""critic_client":"vibe","critic_cost_usd":0.4,"critic_id":"security","critic_model":"vibe-small","#,
                r#""duration_s":12.5,"exit_code":0,"family":"vibe","finding_count":3,"kind":"vibe","#,
                r#""lens_hashes":{"security":"abc"},"lenses":["security"],"mode":"read-only","model":"vibe-large","#,
                r#""outcome":"ok","prompt_hashes":{"contract":"c","preamble":"p","role":"r","schema":"s"},"#,
                r#""recorded_at":"2026-11-03T12:00:00.000Z","repository":"acme/widgets","retry_of":"critic-0","#,
                r#""role":"reviewer","round":2,"run_id":"run-1","spec":"specs/0001.md","stage":"feature-review","#,
                r#""status":"ok","tokens":{"cached":10,"input":100,"output":20},"type":"invocation","#,
                r#""vibe_cost_usd":0.25,"workflow":"feature"}"#
            )
        )
    );
}

#[test]
fn logs_a_failed_run_without_the_fields_it_did_not_report() {
    let home = temp_dir();
    let runtime = runtime_with(env_for(&home));
    let budget = FlowBudget::new(&runtime);
    assert_eq!(budget.log_invocation(&bare_manifest(), &bare_run(), &bare_client(), None).expect("logged"), None);
    let own = crate::locations::ledger_path(&runtime.env).expect("own ledger");
    assert_eq!(
        read(&own),
        format!(
            "{}\n",
            concat!(
                r#"{"charged_usd":0,"client":"agy","critic_id":"triage","family":"agy","flow":"feature","kind":"agy","#,
                r#""model":"gemini","recorded_at":"2026-11-03T12:00:00.000Z","repository":"acme/widgets","round":1,"#,
                r#""run_id":"run-2","stage":"triage","status":"failed","type":"invocation"}"#
            )
        )
    );
    let gateway = model_gateway::locations::ledger_path(&runtime.gateway.env).expect("gateway ledger");
    assert_eq!(
        read(&gateway),
        format!(
            "{}\n",
            concat!(
                r#"{"at":"2026-11-03T12:00:00.000Z","charged_usd":0,"client":"agy","cost_usd":0,"critic_client":"agy","#,
                r#""critic_cost_usd":null,"critic_id":"triage","critic_model":"gemini","duration_s":0,"exit_code":null,"#,
                r#""family":"agy","kind":"agy","mode":"read-only","model":"gemini","outcome":"failed","#,
                r#""recorded_at":"2026-11-03T12:00:00.000Z","repository":"acme/widgets","round":1,"run_id":"run-2","#,
                r#""stage":"triage","status":"failed","type":"invocation","workflow":"feature"}"#
            )
        )
    );
}

#[test]
fn carries_a_null_vibe_cost_where_the_client_reported_none() {
    let home = temp_dir();
    let runtime = runtime_with(env_for(&home));
    let budget = FlowBudget::new(&runtime);
    let run = LedgerRun { vibe_cost_usd: Some(None), ..bare_run() };
    assert_eq!(budget.log_invocation(&bare_manifest(), &run, &bare_client(), None).expect("logged"), None);
    let own = read(&crate::locations::ledger_path(&runtime.env).expect("own ledger"));
    assert!(own.contains(r#""type":"invocation","vibe_cost_usd":null}"#), "{own}");
    let gateway = read(&model_gateway::locations::ledger_path(&runtime.gateway.env).expect("gateway ledger"));
    assert!(gateway.contains(r#""type":"invocation","vibe_cost_usd":null,"workflow":"feature"}"#), "{gateway}");
}

#[test]
fn names_the_gateway_reason_for_each_failure_it_knows() {
    let cases: [(&str, Option<&str>, Option<&str>); 11] = [
        ("ok", Some("timeout"), None),
        ("failed", None, None),
        ("failed", Some("timeout"), Some("timeout")),
        ("failed", Some("unparseable-output"), Some("parse-error")),
        ("failed", Some("empty-output"), Some("parse-error")),
        ("failed", Some("missing-structured-output"), Some("parse-error")),
        ("failed", Some("exit-3"), Some("exit-3")),
        ("failed", Some("exit-"), None),
        ("failed", Some("exit-3x"), None),
        ("failed", Some("config-error"), None),
        ("failed", Some("exit-\u{663}"), None),
    ];
    for (status, failure, expected) in cases {
        let run = LedgerRun { status: status.to_string(), failure: failure.map(str::to_string), ..bare_run() };
        let record = critique_ledger_record(&context(), &run, &client(), AT);
        assert_eq!(record.reason.as_deref(), expected, "{status} {failure:?}");
        assert_eq!(record.outcome, if status == "ok" { "ok" } else { "failed" });
    }
}

#[test]
fn a_gateway_ledger_that_will_not_take_the_record_returns_its_message() {
    let home = temp_dir();
    std::fs::create_dir_all(home.join(".local")).expect("local");
    std::fs::write(home.join(".local").join("state"), "not a directory").expect("state file");
    std::fs::write(home.join("state"), "not a directory").expect("state file");
    let runtime = runtime_with(env_for(&home));
    let budget = FlowBudget::new(&runtime);
    let message = budget.log_invocation(&manifest(), &full_run(), &client(), None).expect("logged");
    let message = message.expect("a message");
    assert!(!message.is_empty());
    let own = read(&crate::locations::ledger_path(&runtime.env).expect("own ledger"));
    assert!(own.starts_with(r#"{"charged_usd":0.5,"#), "{own}");
}

#[test]
fn a_missing_home_is_an_error() {
    let runtime = runtime_with(HashMap::new());
    let budget = FlowBudget::new(&runtime);
    let error = budget.log_invocation(&manifest(), &full_run(), &client(), None).expect_err("no home");
    assert_eq!(error.code, "unexpected");
    assert_eq!(error.message, "required environment variable is missing: HOME");
}
