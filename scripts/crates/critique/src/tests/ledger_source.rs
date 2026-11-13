//! The read half of the flow budget: the ledger readers, the two ledgers
//! merged, and the spend and agy count read back from them. The corpus replay
//! compares every case with what the node runner produced.

use crate::budget::{agy_invocations_on, flow_spend, invocation_lines, spend_by_flow, utc_day, FlowSpend, LoggedInvocation};
use crate::error::ShapeError;
use crate::fields::Fields;
use crate::flow_budget::FlowBudget;
use crate::gateway_ledger::gateway_ledger_lines;
use crate::ledger::{parse_ledger, FlowEvent, LedgerCriticRun, LedgerFinding, LedgerFlowEvent, LedgerRunContext, LedgerSkippedReview};
use crate::ledger_source::{LedgerRead, LedgerSource};
use crate::Runtime;
use model_gateway::json::{self, Object, Value};
use model_gateway::ports::{Clock, FileStamp, Files, FsError};
use std::collections::{HashMap, HashSet};

const AT_MS: i64 = 1_793_707_200_000;
const OWN: &str = "/h/.local/share/agent-kit/critique/ledger.jsonl";
const GATEWAY: &str = "/h/state/agent-kit/model-gateway/ledger.jsonl";

struct Frozen(i64);

impl Clock for Frozen {
    fn now_ms(&self) -> i64 {
        self.0
    }

    fn monotonic_ms(&self) -> f64 {
        0.0
    }
}

/// The files a test puts on the machine: text by path, and the paths that are a directory.
#[derive(Clone, Default)]
struct MemoryFiles {
    files: HashMap<String, String>,
    dirs: HashSet<String>,
}

impl MemoryFiles {
    fn with(entries: &[(&str, &str)]) -> MemoryFiles {
        let mut files = HashMap::new();
        for (path, text) in entries {
            files.insert((*path).to_string(), (*text).to_string());
        }
        MemoryFiles { files, dirs: HashSet::new() }
    }

    fn directory(path: &str) -> MemoryFiles {
        let mut dirs = HashSet::new();
        dirs.insert(path.to_string());
        MemoryFiles { files: HashMap::new(), dirs }
    }
}

fn missing(path: &str) -> FsError {
    FsError { code: "ENOENT".to_string(), message: format!("ENOENT: no such file or directory, open '{path}'") }
}

impl Files for MemoryFiles {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        match self.files.get(path) {
            Some(text) => Ok(text.clone()),
            None if self.dirs.contains(path) => {
                Err(FsError { code: "EISDIR".to_string(), message: format!("EISDIR: illegal operation on a directory, read '{path}'") })
            }
            None => Err(missing(path)),
        }
    }

    fn write_atomic(&self, path: &str, _content: &str) -> Result<(), FsError> {
        Err(missing(path))
    }

    fn mkdir_all(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        Err(missing(path))
    }

    fn remove_file(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn modified_ms(&self, _path: &str) -> Option<i64> {
        None
    }

    fn exists(&self, path: &str) -> Result<bool, FsError> {
        Ok(self.files.contains_key(path) || self.dirs.contains(path))
    }

    fn append(&self, path: &str, _content: &str) -> Result<(), FsError> {
        Err(missing(path))
    }

    fn stamp(&self, _path: &str) -> Option<FileStamp> {
        None
    }

    fn write_private(&self, path: &str, _content: &str) -> Result<(), FsError> {
        Err(missing(path))
    }

    fn write_text(&self, path: &str, _content: &str) -> Result<(), FsError> {
        Err(missing(path))
    }

    fn temp_dir(&self, path: &str, _prefix: &str) -> Result<String, FsError> {
        Err(missing(path))
    }

    fn remove(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }
}

fn runtime_with(files: MemoryFiles, now_ms: i64) -> Runtime {
    let mut env = HashMap::new();
    env.insert("HOME".to_string(), "/h".to_string());
    env.insert("XDG_STATE_HOME".to_string(), "/h/state".to_string());
    runtime_in(env, files, now_ms)
}

fn runtime_in(env: HashMap<String, String>, files: MemoryFiles, now_ms: i64) -> Runtime {
    let mut gateway = model_gateway::default_runtime();
    gateway.env = env.clone();
    gateway.clock = Box::new(Frozen(now_ms));
    gateway.files = Box::new(files.clone());
    Runtime {
        gateway,
        clock: Box::new(Frozen(now_ms)),
        files: Box::new(files),
        dirs: Box::new(crate::dirs::SystemDirs),
        git: Box::new(crate::git::SystemGit { env: env.clone() }),
        env,
        assets_root: "/assets".to_string(),
        stdin: Box::new(String::new),
    }
}

fn text_or_null(value: &Option<String>) -> Value {
    value.as_ref().map_or(Value::Null, |text| Value::str(text))
}

fn number_or_null(value: Option<f64>) -> Value {
    value.map_or(Value::Null, Value::Num)
}

fn string_array(values: &[String]) -> Value {
    Value::Arr(values.iter().map(|value| Value::str(value)).collect())
}

fn string_map(entries: &[(String, String)]) -> Value {
    let mut object = Object::new();
    for (key, value) in entries {
        object.set(key, Value::str(value));
    }
    Value::Obj(object)
}

fn invocation_json(line: &LoggedInvocation) -> Value {
    let mut object = Object::new();
    object.set("charged_usd", Value::Num(line.charged_usd));
    object.set("critic_id", Value::str(&line.critic_id));
    object.set("flow", Value::str(&line.flow));
    object.set("kind", Value::str(&line.kind));
    object.set("recorded_at", Value::str(&line.recorded_at));
    object.set("repository", Value::str(&line.repository));
    if let Some(run_id) = &line.run_id {
        object.set("run_id", Value::str(run_id));
    }
    Value::Obj(object)
}

fn context_into(object: &mut Object, context: &LedgerRunContext) {
    object.set("recorded_at", Value::str(&context.recorded_at));
    object.set("repository", Value::str(&context.repository));
    object.set("run_id", Value::str(&context.run_id));
    if let Some(spec) = &context.spec {
        object.set("spec", Value::str(spec));
    }
    if let Some(flow) = &context.flow {
        object.set("flow", Value::str(flow));
    }
    object.set("round", Value::Num(context.round));
    object.set("stage", Value::str(&context.stage));
}

fn run_json(run: &LedgerCriticRun) -> Value {
    let mut object = Object::new();
    context_into(&mut object, &run.context);
    object.set("client", Value::str(&run.client));
    object.set("cost_usd", number_or_null(run.cost_usd));
    object.set("critic_id", Value::str(&run.critic_id));
    object.set("failure", text_or_null(&run.failure));
    object.set("finding_count", Value::Num(run.finding_count));
    object.set("lens_hashes", string_map(&run.lens_hashes));
    object.set("lenses", string_array(&run.lenses));
    object.set("model", Value::str(&run.model));
    object.set(
        "prompt_hashes",
        json::object(vec![
            ("contract", Value::str(&run.prompt_hashes.contract)),
            ("preamble", Value::str(&run.prompt_hashes.preamble)),
            ("role", Value::str(&run.prompt_hashes.role)),
            ("schema", Value::str(&run.prompt_hashes.schema)),
        ]),
    );
    object.set("role", Value::str(&run.role));
    object.set("status", Value::str(&run.status));
    Value::Obj(object)
}

fn finding_json(finding: &LedgerFinding) -> Value {
    let mut object = Object::new();
    context_into(&mut object, &finding.context);
    object.set("answered", Value::Bool(finding.answered));
    object.set("finding_id", Value::str(&finding.finding_id));
    object.set("pre_verdict", text_or_null(&finding.pre_verdict));
    object.set("raised_by", string_array(&finding.raised_by));
    object.set("severity", Value::str(&finding.severity));
    object.set("verdict", Value::str(&finding.verdict));
    Value::Obj(object)
}

fn skipped_json(review: &LedgerSkippedReview) -> Value {
    json::object(vec![
        ("flow", Value::str(&review.flow)),
        ("recorded_at", Value::str(&review.recorded_at)),
        ("repository", Value::str(&review.repository)),
        ("run_id", Value::str(&review.run_id)),
        ("ticket", Value::str(&review.ticket)),
    ])
}

fn event_json(event: &LedgerFlowEvent) -> Value {
    let mut object = Object::new();
    object.set("flow", Value::str(&event.flow));
    object.set("recorded_at", Value::str(&event.recorded_at));
    object.set("repository", Value::str(&event.repository));
    match &event.event {
        FlowEvent::Grilled => object.set("event", Value::str("grilled")),
        FlowEvent::NoCritique(value) => {
            object.set("event", Value::str("no-critique"));
            object.set("value", Value::Bool(*value));
        }
        FlowEvent::Landed(ticket) => {
            object.set("event", Value::str("landed"));
            object.set("ticket", Value::str(ticket));
        }
        FlowEvent::Answered { run_id, finding_id } => {
            object.set("event", Value::str("answered"));
            object.set("finding_id", Value::str(finding_id));
            object.set("run_id", Value::str(run_id));
        }
        FlowEvent::Draft(draft) => {
            object.set("event", Value::str("draft"));
            object.set("draft", Value::str(draft));
        }
    }
    Value::Obj(object)
}

fn read_json(read: &LedgerRead, spend: &[FlowSpend], agy: usize) -> Value {
    json::object(vec![
        ("agy", Value::count(agy)),
        (
            "read",
            json::object(vec![
                ("invocations", Value::Arr(read.invocations.iter().map(invocation_json).collect())),
                (
                    "ledger",
                    json::object(vec![
                        ("findings", Value::Arr(read.ledger.findings.iter().map(finding_json).collect())),
                        ("flow_events", Value::Arr(read.ledger.flow_events.iter().map(event_json).collect())),
                        ("runs", Value::Arr(read.ledger.runs.iter().map(run_json).collect())),
                        ("skipped_reviews", Value::Arr(read.ledger.skipped_reviews.iter().map(skipped_json).collect())),
                    ]),
                ),
                ("recorded_runs", Value::Arr(read.recorded_runs.iter().map(run_json).collect())),
            ]),
        ),
        (
            "spend",
            Value::Arr(
                spend
                    .iter()
                    .map(|entry| json::object(vec![("flow", Value::str(&entry.flow)), ("spent_usd", Value::Num(entry.spent_usd))]))
                    .collect(),
            ),
        ),
    ])
}

/// The same value with every object's keys in alphabetical order, so two readers' output compares.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Obj(object) => {
            let mut pairs: Vec<(String, Value)> =
                object.entries().into_iter().map(|(key, value)| (key.to_string(), canonical(value))).collect();
            pairs.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = Object::new();
            for (key, value) in pairs {
                sorted.set(&key, value);
            }
            Value::Obj(sorted)
        }
        Value::Arr(items) => Value::Arr(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

fn case_text(case: &Object, key: &str) -> Option<String> {
    match case.get(key) {
        Some(Value::Str(text)) => Some(text.clone()),
        _ => None,
    }
}

fn case_strings(case: &Object, key: &str) -> Vec<String> {
    match case.get(key) {
        Some(Value::Arr(items)) => items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    }
}

fn replay(index: usize, case: &Value) {
    let object = case.as_object().unwrap_or_else(|| panic!("case {index} is not an object"));
    let own = case_text(object, "own");
    let gateway = case_text(object, "gateway");
    let flows = case_strings(object, "flows");
    let repository = case_text(object, "repository").unwrap_or_default();
    let now = model_gateway::timestamps::parse(&case_text(object, "now").unwrap_or_default())
        .unwrap_or_else(|| panic!("case {index} has no instant"));
    let expected = object.get("expected").unwrap_or_else(|| panic!("case {index} has no expected"));
    let mut files = MemoryFiles::default();
    if let Some(own) = &own {
        files.files.insert(OWN.to_string(), own.clone());
    }
    if let Some(gateway) = &gateway {
        files.files.insert(GATEWAY.to_string(), gateway.clone());
    }
    let runtime = runtime_with(files, now);
    let actual = match LedgerSource::new(&runtime).read() {
        Ok(read) => {
            let budget = FlowBudget::new(&runtime);
            let spend = budget.vibe_spend_by_flow(&repository, &flows).expect("the spend of a ledger that read");
            let agy = budget.agy_invocations_today().expect("the agy count of a ledger that read");
            json::object(vec![("ok", read_json(&read, &spend, agy))])
        }
        Err(error) => json::object(vec![(
            "error",
            json::object(vec![("code", Value::str(&error.code)), ("message", Value::str(&error.message))]),
        )]),
    };
    let actual_text = json::compact(&canonical(&actual));
    let expected_text = json::compact(&canonical(expected));
    assert_eq!(
        actual_text, expected_text,
        "case {index} differs\n  own: {own:?}\n  gateway: {gateway:?}\n  expected: {expected_text}\n  actual:   {actual_text}"
    );
}

#[test]
fn ledger_corpus() {
    let text = match std::env::var("CRITIQUE_LEDGER_CORPUS") {
        Ok(path) => std::fs::read_to_string(&path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/ledger-read.json").to_string(),
    };
    let cases = match json::parse(&text) {
        Ok(Value::Arr(items)) => items,
        other => panic!("the corpus is not a JSON array: {other:?}"),
    };
    assert!(!cases.is_empty(), "the corpus holds no case");
    for (index, case) in cases.iter().enumerate() {
        replay(index, case);
    }
}

#[test]
fn nullable_readers_take_null_and_the_value() {
    let value = json::parse(r#"{"a":null,"b":2.5,"c":null,"d":"x","e":{},"f":{"z":"1","0":"zero","a":"2"},"g":{"a":1}}"#).expect("json");
    let fields = Fields::of(&value, "").expect("object");
    assert_eq!(fields.nullable_number("a"), Ok(None));
    assert_eq!(fields.nullable_number("b"), Ok(Some(2.5)));
    assert_eq!(fields.nullable_number("d"), Err(ShapeError("d must be a number".to_string())));
    assert_eq!(fields.nullable_string("c"), Ok(None));
    assert_eq!(fields.nullable_string("d"), Ok(Some("x".to_string())));
    assert_eq!(fields.nullable_string("b"), Err(ShapeError("b must be a non-empty string".to_string())));
    assert_eq!(
        fields.string_map("f"),
        Ok(vec![("0".to_string(), "zero".to_string()), ("z".to_string(), "1".to_string()), ("a".to_string(), "2".to_string())])
    );
    assert_eq!(fields.string_map("e"), Ok(Vec::new()));
    assert_eq!(fields.string_map("b"), Err(ShapeError("b must be a JSON object".to_string())));
    assert_eq!(fields.string_map("g"), Err(ShapeError("g.a must be a non-empty string".to_string())));
}

#[test]
fn invocation_lines_skip_what_is_not_a_launch() {
    let ledger = concat!(
        "{\"type\":\"invocation\",\"charged_usd\":0.5,\"critic_id\":\"c1\",\"flow\":\"f1\",\"kind\":\"vibe\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r1\"}\r\n",
        "\n",
        "not json\n",
        "[1]\n",
        "{\"charged_usd\":1}\n",
        "{\"type\":\"run\"}\n",
        "{\"type\":7}\n",
        "{\"type\":\"invocation\",\"charged_usd\":\"x\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":1,\"critic_id\":\"c2\",\"flow\":\"f2\",\"kind\":\"agy\",\"recorded_at\":\"2026-11-03T01:00:00.000Z\",\"repository\":\"o/r\"}\n",
    );
    let lines = invocation_lines(ledger);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].run_id, Some("r1".to_string()));
    assert_eq!(lines[0].charged_usd, 0.5);
    assert_eq!(lines[1].run_id, None);
    assert_eq!(lines[1].kind, "agy");
}

#[test]
fn spend_and_agy_count_read_the_lines() {
    let lines = invocation_lines(concat!(
        "{\"type\":\"invocation\",\"charged_usd\":0.5,\"critic_id\":\"c1\",\"flow\":\"f1\",\"kind\":\"vibe\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":0.25,\"critic_id\":\"c2\",\"flow\":\"f1\",\"kind\":\"vibe\",\"recorded_at\":\"2026-11-03T01:00:00.000Z\",\"repository\":\"o/r\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":9,\"critic_id\":\"c3\",\"flow\":\"f1\",\"kind\":\"agy\",\"recorded_at\":\"2026-11-03T02:00:00.000Z\",\"repository\":\"o/r\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":9,\"critic_id\":\"c4\",\"flow\":\"f1\",\"kind\":\"vibe\",\"recorded_at\":\"2026-11-02T02:00:00.000Z\",\"repository\":\"other/r\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":9,\"critic_id\":\"c5\",\"flow\":\"f2\",\"kind\":\"agy\",\"recorded_at\":\"2026-11-02T02:00:00.000Z\",\"repository\":\"o/r\"}\n",
    ));
    assert_eq!(flow_spend(&lines, "o/r", "f1"), 0.75);
    assert_eq!(flow_spend(&lines, "o/r", "f2"), 0.0);
    assert_eq!(
        spend_by_flow(&lines, "o/r", &["f1".to_string(), "f2".to_string()]),
        vec![FlowSpend { flow: "f1".to_string(), spent_usd: 0.75 }, FlowSpend { flow: "f2".to_string(), spent_usd: 0.0 }]
    );
    assert_eq!(utc_day(AT_MS), "2026-11-03");
    assert_eq!(agy_invocations_on(&lines, AT_MS), 1);
    assert_eq!(agy_invocations_on(&lines, AT_MS - 86_400_000), 1);
}

#[test]
fn parse_ledger_reads_every_line_type() {
    let text = concat!(
        "\n",
        "{\"type\":\"run\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r1\",\"stage\":\"spec\",\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":null,\"finding_count\":2,\"lens_hashes\":{\"a\":\"b\"},\"lenses\":[\"a\"],\"model\":\"m\",\"prompt_hashes\":{\"contract\":\"c\",\"preamble\":\"p\",\"role\":\"r\",\"schema\":\"s\"},\"role\":\"r\",\"status\":\"ok\"}\n",
        "{\"type\":\"run\",\"recorded_at\":\"2026-11-04T00:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r2\",\"stage\":\"tickets\",\"flow\":\"f1\",\"status\":\"skipped-low-risk\",\"ticket\":\"t1\"}\n",
        "{\"type\":\"finding\",\"recorded_at\":\"2026-11-02T00:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r3\",\"stage\":\"plan\",\"spec\":\"s.md\",\"flow\":\"f1\",\"round\":2,\"finding_id\":\"f-1\",\"severity\":\"critical\",\"raised_by\":[\"c1\"],\"verdict\":\"hint\",\"pre_verdict\":null,\"answered\":true}\n",
        "{\"type\":\"flow\",\"flow\":\"f1\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"event\":\"grilled\"}\n",
        "{\"type\":\"flow\",\"flow\":\"f1\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"event\":\"no-critique\",\"value\":true}\n",
        "{\"type\":\"flow\",\"flow\":\"f1\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"event\":\"landed\",\"ticket\":\"t1\"}\n",
        "{\"type\":\"flow\",\"flow\":\"f1\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"event\":\"answered\",\"finding_id\":\"f-1\",\"run_id\":\"r1\"}\n",
        "{\"type\":\"flow\",\"flow\":\"f1\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"event\":\"draft\",\"draft\":\"d1\"}\n",
        "{\"type\":\"delegation\"}\n",
        "  \n",
    );
    let ledger = parse_ledger(text).expect("a ledger");
    assert_eq!(ledger.runs.len(), 1);
    let run = &ledger.runs[0];
    assert_eq!(run.context.run_id, "r1");
    assert_eq!(run.context.spec, None);
    assert_eq!(run.context.flow, None);
    assert_eq!(run.context.round, 1.0);
    assert_eq!(run.cost_usd, None);
    assert_eq!(run.failure, None);
    assert_eq!(run.finding_count, 2.0);
    assert_eq!(run.lens_hashes, vec![("a".to_string(), "b".to_string())]);
    assert_eq!(run.lenses, vec!["a".to_string()]);
    assert_eq!(run.prompt_hashes.contract, "c");
    assert_eq!(run.status, "ok");
    assert_eq!(ledger.skipped_reviews.len(), 1);
    assert_eq!(ledger.skipped_reviews[0].ticket, "t1");
    assert_eq!(ledger.findings.len(), 1);
    let finding = &ledger.findings[0];
    assert_eq!(finding.context.spec, Some("s.md".to_string()));
    assert_eq!(finding.context.flow, Some("f1".to_string()));
    assert_eq!(finding.context.round, 2.0);
    assert_eq!(finding.pre_verdict, None);
    assert!(finding.answered);
    assert_eq!(finding.verdict, "hint");
    assert_eq!(
        ledger.flow_events.iter().map(|event| event.event.clone()).collect::<Vec<FlowEvent>>(),
        vec![
            FlowEvent::Grilled,
            FlowEvent::NoCritique(true),
            FlowEvent::Landed("t1".to_string()),
            FlowEvent::Answered { run_id: "r1".to_string(), finding_id: "f-1".to_string() },
            FlowEvent::Draft("d1".to_string()),
        ]
    );
}

fn shape(text: &str) -> String {
    parse_ledger(text).expect_err("a shape error").0
}

#[test]
fn parse_ledger_names_the_line_and_the_field() {
    assert_eq!(shape("nope"), "line 1 is not JSON");
    assert_eq!(shape("\n[1]"), "line 2 must be a JSON object");
    assert_eq!(shape("{}"), "line 1.type must be a non-empty string");
    assert_eq!(shape("{\"type\":\"run\"}"), "line 1.prompt_hashes must be a JSON object");
    assert_eq!(shape("{\"type\":\"run\",\"prompt_hashes\":{}}"), "line 1.recorded_at must be a non-empty string");
    assert_eq!(shape("{\"type\":\"run\",\"prompt_hashes\":{},\"recorded_at\":\"a\"}"), "line 1.repository must be a non-empty string");
    assert_eq!(
        shape("{\"type\":\"run\",\"prompt_hashes\":{},\"recorded_at\":\"a\",\"repository\":\"b\"}"),
        "line 1.run_id must be a non-empty string"
    );
    assert_eq!(
        shape("{\"type\":\"run\",\"prompt_hashes\":{},\"recorded_at\":\"a\",\"repository\":\"b\",\"run_id\":\"c\",\"spec\":7}"),
        "line 1.spec must be a non-empty string"
    );
    assert_eq!(
        shape("{\"type\":\"run\",\"prompt_hashes\":{},\"recorded_at\":\"a\",\"repository\":\"b\",\"run_id\":\"c\",\"flow\":7}"),
        "line 1.flow must be a non-empty string"
    );
    assert_eq!(
        shape("{\"type\":\"run\",\"prompt_hashes\":{},\"recorded_at\":\"a\",\"repository\":\"b\",\"run_id\":\"c\",\"round\":\"x\"}"),
        "line 1.round must be a number"
    );
    assert_eq!(
        shape("{\"type\":\"run\",\"prompt_hashes\":{},\"recorded_at\":\"a\",\"repository\":\"b\",\"run_id\":\"c\",\"stage\":\"nope\"}"),
        "line 1.stage must be one of decisions, spec, tickets, ticket-review, feature-review, adr-review, decision-recovery, architecture-review, plan"
    );
    let context = "\"recorded_at\":\"a\",\"repository\":\"b\",\"run_id\":\"c\",\"stage\":\"spec\"";
    assert_eq!(shape(&format!("{{\"type\":\"run\",\"prompt_hashes\":{{}},{context}}}")), "line 1.client must be a non-empty string");
    assert_eq!(
        shape(&format!("{{\"type\":\"run\",\"prompt_hashes\":{{}},{context},\"client\":\"vibe\",\"cost_usd\":\"x\"}}")),
        "line 1.cost_usd must be a number"
    );
    assert_eq!(
        shape(&format!("{{\"type\":\"run\",\"prompt_hashes\":{{}},{context},\"client\":\"vibe\",\"cost_usd\":null}}")),
        "line 1.critic_id must be a non-empty string"
    );
    assert_eq!(
        shape(&format!(
            "{{\"type\":\"run\",\"prompt_hashes\":{{}},{context},\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":7}}"
        )),
        "line 1.failure must be a non-empty string"
    );
    assert_eq!(
        shape(&format!(
            "{{\"type\":\"run\",\"prompt_hashes\":{{}},{context},\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":null,\"finding_count\":\"x\"}}"
        )),
        "line 1.finding_count must be a number"
    );
    assert_eq!(
        shape(&format!(
            "{{\"type\":\"run\",\"prompt_hashes\":{{}},{context},\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":null,\"finding_count\":0,\"lens_hashes\":7}}"
        )),
        "line 1.lens_hashes must be a JSON object"
    );
    assert_eq!(
        shape(&format!(
            "{{\"type\":\"run\",\"prompt_hashes\":{{}},{context},\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":null,\"finding_count\":0,\"lens_hashes\":{{}},\"lenses\":7}}"
        )),
        "line 1.lenses must be a list"
    );
    assert_eq!(
        shape(&format!(
            "{{\"type\":\"run\",\"prompt_hashes\":{{}},{context},\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":null,\"finding_count\":0,\"lens_hashes\":{{}},\"lenses\":[],\"model\":7}}"
        )),
        "line 1.model must be a non-empty string"
    );
    assert_eq!(
        shape(&format!(
            "{{\"type\":\"run\",\"prompt_hashes\":{{}},{context},\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":null,\"finding_count\":0,\"lens_hashes\":{{}},\"lenses\":[],\"model\":\"m\"}}"
        )),
        "line 1.prompt_hashes.contract must be a non-empty string"
    );
    assert_eq!(
        shape(&format!(
            "{{\"type\":\"run\",\"prompt_hashes\":{{\"contract\":\"c\",\"preamble\":\"p\",\"role\":\"r\",\"schema\":\"s\"}},{context},\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":null,\"finding_count\":0,\"lens_hashes\":{{}},\"lenses\":[],\"model\":\"m\"}}"
        )),
        "line 1.role must be a non-empty string"
    );
    assert_eq!(
        shape(&format!(
            "{{\"type\":\"run\",\"prompt_hashes\":{{\"contract\":\"c\",\"preamble\":\"p\",\"role\":\"r\",\"schema\":\"s\"}},{context},\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":null,\"finding_count\":0,\"lens_hashes\":{{}},\"lenses\":[],\"model\":\"m\",\"role\":\"r\",\"status\":\"nope\"}}"
        )),
        "line 1.status must be one of ok, failed, violated-readonly, budget-skip"
    );
    assert_eq!(
        shape(&format!("{{\"type\":\"run\",{context},\"status\":7}}")),
        "line 1.status must be a non-empty string"
    );
    assert_eq!(
        shape(&format!("{{\"type\":\"run\",{context},\"status\":\"skipped-low-risk\"}}")),
        "line 1.flow must be a non-empty string"
    );
    assert_eq!(
        shape(&format!("{{\"type\":\"run\",{context},\"status\":\"skipped-low-risk\",\"flow\":\"f\"}}")),
        "line 1.ticket must be a non-empty string"
    );
    assert_eq!(shape("{\"type\":\"finding\"}"), "line 1.recorded_at must be a non-empty string");
    assert_eq!(
        shape(&format!("{{\"type\":\"finding\",{context},\"answered\":7}}")),
        "line 1.answered must be true or false"
    );
    assert_eq!(
        shape(&format!("{{\"type\":\"finding\",{context},\"answered\":true}}")),
        "line 1.finding_id must be a non-empty string"
    );
    assert_eq!(
        shape(&format!("{{\"type\":\"finding\",{context},\"finding_id\":\"f\"}}")),
        "line 1.pre_verdict must be a non-empty string"
    );
    assert_eq!(
        shape(&format!("{{\"type\":\"finding\",{context},\"finding_id\":\"f\",\"pre_verdict\":null}}")),
        "line 1.severity must be one of critical, major, minor"
    );
    assert_eq!(
        shape(&format!("{{\"type\":\"finding\",{context},\"finding_id\":\"f\",\"pre_verdict\":null,\"severity\":\"major\"}}")),
        "line 1.raised_by must be a list"
    );
    assert_eq!(
        shape(&format!(
            "{{\"type\":\"finding\",{context},\"finding_id\":\"f\",\"pre_verdict\":null,\"severity\":\"major\",\"raised_by\":[]}}"
        )),
        "line 1.verdict must be one of accepted, rejected, needs-human, hint"
    );
    assert_eq!(shape("{\"type\":\"flow\"}"), "line 1.flow must be a non-empty string");
    assert_eq!(shape("{\"type\":\"flow\",\"flow\":\"f\"}"), "line 1.recorded_at must be a non-empty string");
    assert_eq!(
        shape("{\"type\":\"flow\",\"flow\":\"f\",\"recorded_at\":\"a\"}"),
        "line 1.repository must be a non-empty string"
    );
    assert_eq!(
        shape("{\"type\":\"flow\",\"flow\":\"f\",\"recorded_at\":\"a\",\"repository\":\"b\"}"),
        "line 1.event must be one of grilled, no-critique, landed, answered, draft"
    );
    let event = "\"type\":\"flow\",\"flow\":\"f\",\"recorded_at\":\"a\",\"repository\":\"b\"";
    assert_eq!(shape(&format!("{{{event},\"event\":\"no-critique\"}}")), "line 1.value must be true or false");
    assert_eq!(shape(&format!("{{{event},\"event\":\"landed\"}}")), "line 1.ticket must be a non-empty string");
    assert_eq!(shape(&format!("{{{event},\"event\":\"answered\"}}")), "line 1.finding_id must be a non-empty string");
    assert_eq!(
        shape(&format!("{{{event},\"event\":\"answered\",\"finding_id\":\"f\"}}")),
        "line 1.run_id must be a non-empty string"
    );
    assert_eq!(shape(&format!("{{{event},\"event\":\"draft\"}}")), "line 1.draft must be a non-empty string");
}

#[test]
fn gateway_ledger_reads_records_and_skips_the_rest() {
    let text = concat!(
        "\n",
        "not json\n",
        "[1]\n",
        "{\"type\":\"delegation\"}\n",
        "{\"type\":7}\n",
        "{\"type\":\"invocation\",\"charged_usd\":0.1,\"critic_id\":\"c1\",\"kind\":\"agy\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r1\",\"workflow\":\"f1\",\"round\":1,\"stage\":\"spec\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":0.2,\"critic_id\":\"c2\",\"kind\":\"vibe\",\"recorded_at\":\"2026-11-03T01:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r2\",\"workflow\":\"f1\",\"round\":2,\"stage\":\"plan\",\"spec\":\"s.md\",\"prompt_hashes\":{\"contract\":\"c\",\"preamble\":\"p\",\"role\":\"r\",\"schema\":\"s\"},\"critic_client\":\"vibe\",\"critic_cost_usd\":null,\"critic_model\":\"m\",\"status\":\"ok\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":0.3,\"critic_id\":\"c3\",\"kind\":\"codex\",\"recorded_at\":\"2026-11-03T02:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r3\",\"workflow\":\"f1\",\"round\":3,\"stage\":\"plan\",\"prompt_hashes\":{\"contract\":\"c\",\"preamble\":\"p\",\"role\":\"r\",\"schema\":\"s\"},\"critic_client\":\"vibe\",\"critic_cost_usd\":0.25,\"critic_model\":\"m\",\"status\":\"failed\",\"failure\":\"timeout\",\"finding_count\":4,\"lens_hashes\":{\"a\":\"b\"},\"lenses\":[\"a\"],\"role\":\"r\"}\n",
    );
    let lines = gateway_ledger_lines(text);
    assert_eq!(lines.invocations.len(), 3);
    assert_eq!(lines.runs.len(), 2);
    assert_eq!(lines.invocations[0].run_id, Some("r1".to_string()));
    assert_eq!(lines.invocations[0].flow, "f1");
    let bare = &lines.runs[0];
    assert_eq!(bare.context.spec, Some("s.md".to_string()));
    assert_eq!(bare.context.flow, Some("f1".to_string()));
    assert_eq!(bare.cost_usd, None);
    assert_eq!(bare.failure, None);
    assert_eq!(bare.finding_count, 0.0);
    assert_eq!(bare.lens_hashes, Vec::new());
    assert_eq!(bare.lenses, Vec::<String>::new());
    assert_eq!(bare.role, "");
    let full = &lines.runs[1];
    assert_eq!(full.context.spec, None);
    assert_eq!(full.cost_usd, Some(0.25));
    assert_eq!(full.failure, Some("timeout".to_string()));
    assert_eq!(full.finding_count, 4.0);
    assert_eq!(full.lens_hashes, vec![("a".to_string(), "b".to_string())]);
    assert_eq!(full.lenses, vec!["a".to_string()]);
    assert_eq!(full.role, "r");
    assert_eq!(full.status, "failed");
}

#[test]
fn gateway_ledger_skips_a_record_of_the_wrong_shape() {
    let text = concat!(
        "{\"type\":\"invocation\",\"charged_usd\":0.1,\"critic_id\":\"c1\",\"kind\":\"agy\",\"recorded_at\":\"a\",\"repository\":\"o/r\",\"run_id\":\"r1\",\"workflow\":\"f1\",\"round\":1,\"stage\":\"nope\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":0.1,\"critic_id\":\"c1\",\"kind\":\"agy\",\"recorded_at\":\"a\",\"repository\":\"o/r\",\"run_id\":\"r1\",\"workflow\":\"f1\",\"round\":1,\"stage\":\"spec\",\"prompt_hashes\":{}}\n",
    );
    let lines = gateway_ledger_lines(text);
    assert_eq!(lines.invocations.len(), 0);
    assert_eq!(lines.runs.len(), 0);
}

fn own_ledger() -> &'static str {
    concat!(
        "{\"type\":\"invocation\",\"charged_usd\":0.5,\"critic_id\":\"c1\",\"flow\":\"f1\",\"kind\":\"vibe\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r1\"}\n",
        "{\"type\":\"run\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r1\",\"stage\":\"spec\",\"client\":\"vibe\",\"cost_usd\":null,\"critic_id\":\"c1\",\"failure\":null,\"finding_count\":0,\"lens_hashes\":{},\"lenses\":[],\"model\":\"m\",\"prompt_hashes\":{\"contract\":\"c\",\"preamble\":\"p\",\"role\":\"r\",\"schema\":\"s\"},\"role\":\"r\",\"status\":\"ok\"}\n",
    )
}

fn gateway_ledger() -> &'static str {
    concat!(
        "{\"type\":\"invocation\",\"charged_usd\":0.5,\"critic_id\":\"c1\",\"kind\":\"vibe\",\"recorded_at\":\"2026-11-03T00:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r1\",\"workflow\":\"f1\",\"round\":1,\"stage\":\"spec\",\"prompt_hashes\":{\"contract\":\"c\",\"preamble\":\"p\",\"role\":\"r\",\"schema\":\"s\"},\"critic_client\":\"vibe\",\"critic_cost_usd\":null,\"critic_model\":\"m\",\"status\":\"ok\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":0.25,\"critic_id\":\"c2\",\"kind\":\"agy\",\"recorded_at\":\"2026-11-03T01:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r2\",\"workflow\":\"f1\",\"round\":1,\"stage\":\"plan\"}\n",
        "{\"type\":\"invocation\",\"charged_usd\":0.25,\"critic_id\":\"c3\",\"kind\":\"vibe\",\"recorded_at\":\"2026-11-03T01:00:00.000Z\",\"repository\":\"o/r\",\"run_id\":\"r3\",\"workflow\":\"f1\",\"round\":1,\"stage\":\"plan\",\"prompt_hashes\":{\"contract\":\"c\",\"preamble\":\"p\",\"role\":\"r\",\"schema\":\"s\"},\"critic_client\":\"vibe\",\"critic_cost_usd\":null,\"critic_model\":\"m\",\"status\":\"ok\"}\n",
    )
}

#[test]
fn read_merges_both_ledgers_once() {
    let runtime = runtime_with(MemoryFiles::with(&[(OWN, own_ledger()), (GATEWAY, gateway_ledger())]), AT_MS);
    let read = LedgerSource::new(&runtime).read().expect("a ledger");
    assert_eq!(read.invocations.len(), 3);
    assert_eq!(read.invocations[0].critic_id, "c1");
    assert_eq!(read.invocations[1].critic_id, "c2");
    assert_eq!(read.invocations[2].critic_id, "c3");
    assert_eq!(read.ledger.runs.len(), 2);
    assert_eq!(read.ledger.runs[0].critic_id, "c1");
    assert_eq!(read.ledger.runs[1].critic_id, "c3");
    assert_eq!(read.recorded_runs.len(), 1);
    assert_eq!(read.recorded_runs[0].critic_id, "c1");
}

#[test]
fn read_takes_an_empty_own_ledger_for_an_empty_one() {
    let runtime = runtime_with(MemoryFiles::with(&[(GATEWAY, gateway_ledger())]), AT_MS);
    let read = LedgerSource::new(&runtime).read().expect("a ledger");
    assert_eq!(read.ledger.findings, Vec::new());
    assert_eq!(read.ledger.flow_events, Vec::new());
    assert_eq!(read.ledger.skipped_reviews, Vec::new());
    assert_eq!(read.recorded_runs, Vec::new());
    assert_eq!(read.invocations.len(), 3);
    assert_eq!(read.ledger.runs.len(), 2);
}

#[test]
fn read_reports_an_unreadable_own_ledger() {
    let runtime = runtime_with(MemoryFiles::directory(OWN), AT_MS);
    let error = LedgerSource::new(&runtime).read().expect_err("a directory");
    assert_eq!(error.code, "unexpected");
    assert_eq!(error.message, format!("EISDIR: illegal operation on a directory, read '{OWN}'"));
}

#[test]
fn read_reports_an_unreadable_gateway_ledger() {
    let runtime = runtime_with(MemoryFiles::directory(GATEWAY), AT_MS);
    let error = LedgerSource::new(&runtime).read().expect_err("a directory");
    assert_eq!(error.code, "unexpected");
    assert_eq!(error.message, format!("EISDIR: illegal operation on a directory, read '{GATEWAY}'"));
}

#[test]
fn read_reports_a_missing_home() {
    let runtime = runtime_in(HashMap::new(), MemoryFiles::default(), AT_MS);
    let error = LedgerSource::new(&runtime).read().expect_err("no home");
    assert_eq!(error.code, "unexpected");
    assert_eq!(error.message, "required environment variable is missing: HOME");
}

#[test]
fn read_reports_a_gateway_state_directory_that_cannot_be_named() {
    let mut runtime = runtime_with(MemoryFiles::default(), AT_MS);
    runtime.gateway.env = HashMap::new();
    let error = LedgerSource::new(&runtime).read().expect_err("no gateway home");
    assert_eq!(error.code, "unexpected");
    assert_eq!(error.message, "required environment variable is missing: HOME");
}

#[test]
fn read_reports_an_invalid_own_ledger() {
    let runtime = runtime_with(MemoryFiles::with(&[(OWN, "nope\n")]), AT_MS);
    let error = LedgerSource::new(&runtime).read().expect_err("not JSON");
    assert_eq!(error.code, "invalid-ledger");
    assert_eq!(error.message, format!("{OWN}: line 1 is not JSON"));
}

#[test]
fn flow_budget_reads_spend_and_agy_from_both_ledgers() {
    let runtime = runtime_with(MemoryFiles::with(&[(OWN, own_ledger()), (GATEWAY, gateway_ledger())]), AT_MS);
    let budget = FlowBudget::new(&runtime);
    assert_eq!(
        budget.vibe_spend_by_flow("o/r", &["f1".to_string(), "f2".to_string()]).expect("spend"),
        vec![FlowSpend { flow: "f1".to_string(), spent_usd: 0.75 }, FlowSpend { flow: "f2".to_string(), spent_usd: 0.0 }]
    );
    assert_eq!(budget.agy_invocations_today().expect("agy"), 1);
}
