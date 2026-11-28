//! Tests for `export`: the OTLP request a run's log becomes, where it goes, and what never leaves the machine.

use super::machine::{FakeProcess, Machine, DEFAULTS};
use crate::json::{parse, Value};
use crate::otlp::{export, parse_args, run, ExportOptions};
use crate::ports::{Process, ProcessCommand, ProcessResult};
use crate::run_events::{RunEvents, RunIdentity, RunStart};
use std::sync::{Arc, Mutex};

/// A curl that records the config it was given and the body that config names, and answers as told.
#[derive(Clone)]
struct Collector {
    seen: Arc<Mutex<Vec<(String, String)>>>,
    answer: Arc<Mutex<ProcessResult>>,
    /// Whether the program cannot be started at all.
    missing: bool,
}

impl Collector {
    fn new(exit_code: i32, timed_out: bool, stderr: &str) -> Collector {
        Collector { missing: false, seen: Arc::default(), answer: Arc::new(Mutex::new(ProcessResult { stdout: String::new(), stderr: stderr.into(), exit_code, timed_out })) }
    }
}

impl Process for Collector {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Err("not used".into())
    }
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        assert_eq!(command.command, "curl");
        if self.missing {
            return Err("curl is not installed".into());
        }
        assert!(command.args.iter().all(|arg| !arg.contains("secret")), "no credential in the arguments: {:?}", command.args);
        let config = command.stdin.clone().unwrap_or_default();
        let path = config.lines().find_map(|line| line.strip_prefix("data-binary = \"@")).map(|rest| rest.trim_end_matches('"').to_string()).expect("a body file");
        let body = std::fs::read_to_string(&path).expect("the body is there while curl runs");
        self.seen.lock().expect("seen").push((config, body));
        Ok(self.answer.lock().expect("answer").clone())
    }
}

fn machine_with(collector: &Collector) -> Machine {
    Machine::with_process(DEFAULTS, super::machine::now_ms(), FakeProcess::default(), Box::new(collector.clone()))
}

fn start<'a>(machine: &'a Machine, id: &str, workflow: &str) -> RunEvents<'a> {
    let dir = format!("{}/state/agent-kit/model-gateway/runs/{id}", machine.sandbox.text());
    std::fs::create_dir_all(&dir).expect("run dir");
    let identity = RunIdentity { run_id: id.into(), workflow: workflow.into(), client: "pi".into(), model: "free/hy4-flash".into(), family: "tencent".into() };
    let index = crate::locations::run_index_path(&machine.runtime.env).expect("index path");
    RunEvents::start(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &dir, identity, &RunStart { mode: "read-only", time_limit_s: 900.0 }).indexed(Some(&index))
}

/// Four runs; the last is still going, so the guard that is returned keeps it so.
fn runs(machine: &Machine) -> RunEvents<'_> {
    let ok = start(machine, "r-ok", "a");
    ok.record("tool.started", parse(r#"{"gen_ai.tool.call.id":"c1","gen_ai.tool.name":"Read","input":"SECRET FILE TEXT"}"#).expect("JSON"));
    ok.record("message", parse(r#"{"role":"assistant","text":"SECRET ANSWER","nested":{"a":1},"none":null,"flag":true,"cost":0.5}"#).expect("JSON"));
    ok.record("usage", parse(r#"{"gen_ai.usage.input_tokens":120,"gen_ai.usage.output_tokens":7}"#).expect("JSON"));
    ok.finish(parse(r#"{"status":"ok","reason":null,"exit_code":0,"gen_ai.usage.input_tokens":120}"#).expect("JSON"));
    let failed = start(machine, "r-bad", "b");
    failed.finish(parse(r#"{"status":"failed","reason":"harness error"}"#).expect("JSON"));
    let odd = start(machine, "r-odd", "b");
    odd.finish(parse(r#"{"status":"cancelled"}"#).expect("JSON"));
    start(machine, "r-run", "b")
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

fn spans_of(body: &str) -> Vec<crate::json::Object> {
    let request = parse(body).expect("JSON body");
    let spans = request.as_object().and_then(|r| r.get("resourceSpans")).and_then(|v| match v { Value::Arr(a) => a[0].as_object().cloned(), _ => None }).and_then(|r| r.get("scopeSpans").cloned()).expect("scopeSpans");
    let Value::Arr(scopes) = spans else { panic!("scopeSpans") };
    let Some(Value::Arr(spans)) = scopes[0].as_object().and_then(|s| s.get("spans")).cloned() else { panic!("spans") };
    spans.iter().filter_map(|s| s.as_object().cloned()).collect()
}

fn attr(attributes: &[Value], key: &str) -> Option<String> {
    attributes.iter().filter_map(Value::as_object).find(|a| a.get("key").and_then(Value::as_str) == Some(key)).map(|a| crate::json::compact(a.get("value").expect("value")))
}

fn list(object: &crate::json::Object, key: &str) -> Vec<Value> {
    match object.get(key) {
        Some(Value::Arr(items)) => items.clone(),
        _ => Vec::new(),
    }
}

#[test]
fn a_run_becomes_one_span_with_its_events_and_the_gen_ai_attributes_unchanged() {
    let file = Collector::new(0, false, "");
    let machine = machine_with(&file);
    let _going = runs(&machine);
    let target = format!("{}/out/traces.jsonl", machine.sandbox.text());
    let result = export(&ExportOptions { file: Some(target.clone()), run: Some("r-ok".into()), ..ExportOptions::default() }, &machine.runtime).expect("exported");
    assert_eq!(crate::json::compact(&result), format!(r#"{{"status":"ok","spans":1,"sent_to":[{}]}}"#, crate::json::compact(&Value::str(&target))));
    assert!(file.seen.lock().expect("seen").is_empty(), "no endpoint, no post");
    let text = std::fs::read_to_string(&target).expect("the file");
    assert_eq!(text.lines().count(), 1);
    assert!(!text.contains("SECRET"), "message and tool text stay on the machine: {text}");
    let spans = spans_of(text.trim());
    let span = &spans[0];
    assert_eq!(span.get("name").and_then(Value::as_str), Some("invoke_agent pi"));
    assert_eq!(span.get("traceId").and_then(Value::as_str).map(str::len), Some(32));
    assert_eq!(span.get("spanId").and_then(Value::as_str).map(str::len), Some(16));
    assert_eq!(crate::json::compact(span.get("status").expect("status")), r#"{"code":1}"#);
    let attributes = list(span, "attributes");
    assert_eq!(attr(&attributes, "gen_ai.request.model").as_deref(), Some(r#"{"stringValue":"free/hy4-flash"}"#));
    assert_eq!(attr(&attributes, "gen_ai.system").as_deref(), Some(r#"{"stringValue":"tencent"}"#));
    assert_eq!(attr(&attributes, "gen_ai.agent.name").as_deref(), Some(r#"{"stringValue":"pi"}"#));
    assert_eq!(attr(&attributes, "gen_ai.operation.name").as_deref(), Some(r#"{"stringValue":"invoke_agent"}"#));
    assert_eq!(attr(&attributes, "gen_ai.usage.input_tokens").as_deref(), Some(r#"{"intValue":"120"}"#));
    assert_eq!(attr(&attributes, "gateway.run_id").as_deref(), Some(r#"{"stringValue":"r-ok"}"#));
    let events = list(span, "events");
    let names: Vec<String> = events.iter().filter_map(Value::as_object).filter_map(|e| e.get("name").and_then(Value::as_str).map(str::to_string)).collect();
    assert_eq!(names, ["run.started", "tool.started", "message", "usage", "usage", "run.finished"]);
    let tool = list(events[1].as_object().expect("event"), "attributes");
    assert_eq!(attr(&tool, "gen_ai.tool.name").as_deref(), Some(r#"{"stringValue":"Read"}"#));
    assert_eq!(attr(&tool, "gen_ai.tool.call.id").as_deref(), Some(r#"{"stringValue":"c1"}"#));
    assert_eq!(attr(&tool, "gateway.input"), None);
    let message = list(events[2].as_object().expect("event"), "attributes");
    assert_eq!(attr(&message, "gateway.nested").as_deref(), Some(r#"{"stringValue":"{\"a\":1}"}"#));
    assert_eq!(attr(&message, "gateway.flag").as_deref(), Some(r#"{"boolValue":true}"#));
    assert_eq!(attr(&message, "gateway.cost").as_deref(), Some(r#"{"doubleValue":0.5}"#));
    assert_eq!(attr(&message, "gateway.none"), None);
    let usage = list(events[3].as_object().expect("event"), "attributes");
    assert_eq!(attr(&usage, "gen_ai.usage.output_tokens").as_deref(), Some(r#"{"intValue":"7"}"#));
}

#[test]
fn a_failed_a_cancelled_and_a_running_run_have_their_status() {
    let machine = machine_with(&Collector::new(0, false, ""));
    let _going = runs(&machine);
    let target = format!("{}/t.jsonl", machine.sandbox.text());
    export(&ExportOptions { file: Some(target.clone()), workflow: Some("b".into()), ..ExportOptions::default() }, &machine.runtime).expect("exported");
    let spans = spans_of(std::fs::read_to_string(&target).expect("file").trim());
    let by_run = |id: &str| spans.iter().find(|span| attr(&list(span, "attributes"), "gateway.run_id").as_deref() == Some(&format!(r#"{{"stringValue":"{id}"}}"#))).expect("span");
    assert_eq!(crate::json::compact(by_run("r-bad").get("status").expect("status")), r#"{"code":2,"message":"harness error"}"#);
    assert_eq!(crate::json::compact(by_run("r-odd").get("status").expect("status")), r#"{"code":2,"message":"cancelled"}"#);
    assert_eq!(crate::json::compact(by_run("r-run").get("status").expect("status")), r#"{"code":0}"#);
    assert_eq!(spans.len(), 3);
}

#[test]
fn the_request_is_posted_to_the_endpoint_with_the_headers_in_the_config_not_the_arguments() {
    let collector = Collector::new(0, false, "");
    let mut machine = machine_with(&collector);
    runs(&machine);
    machine.runtime.env.insert("OTEL_EXPORTER_OTLP_ENDPOINT".into(), "http://localhost:4318/".into());
    machine.runtime.env.insert("OTEL_EXPORTER_OTLP_HEADERS".into(), "authorization=Bearer secret, x-team = a\"b,junk".into());
    let result = export(&ExportOptions { run: Some("r-ok".into()), ..ExportOptions::default() }, &machine.runtime).expect("exported");
    assert!(crate::json::compact(&result).contains("http://localhost:4318/v1/traces"));
    let seen = collector.seen.lock().expect("seen");
    let (config, body) = &seen[0];
    assert!(config.contains(r#"url = "http://localhost:4318/v1/traces""#) && config.contains(r#"header = "authorization: Bearer secret""#) && config.contains(r#"header = "x-team: a\"b""#), "{config}");
    assert!(!config.contains("junk"));
    assert_eq!(spans_of(body).len(), 1);
    drop(seen);
    let runs_dir = crate::locations::runs_dir(&machine.runtime.env).expect("runs dir");
    let leftovers: Vec<String> = std::fs::read_dir(&runs_dir).expect("runs").filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(".otlp-")).collect();
    assert!(leftovers.is_empty(), "the body file is removed: {leftovers:?}");
}

#[test]
fn the_endpoint_comes_from_the_option_then_the_traces_variable_then_the_base_variable() {
    let collector = Collector::new(0, false, "");
    let mut machine = machine_with(&collector);
    runs(&machine);
    machine.runtime.env.insert("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT".into(), "http://traces.example/x".into());
    machine.runtime.env.insert("OTEL_EXPORTER_OTLP_ENDPOINT".into(), "http://base.example".into());
    export(&ExportOptions::default(), &machine.runtime).expect("traces variable");
    export(&ExportOptions { endpoint: Some("http://flag.example/t".into()), ..ExportOptions::default() }, &machine.runtime).expect("flag");
    machine.runtime.env.insert("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT".into(), String::new());
    export(&ExportOptions::default(), &machine.runtime).expect("base variable");
    let urls: Vec<String> = collector.seen.lock().expect("seen").iter().map(|(config, _)| config.lines().next().unwrap_or_default().to_string()).collect();
    assert_eq!(urls, [r#"url = "http://traces.example/x""#, r#"url = "http://flag.example/t""#, r#"url = "http://base.example/v1/traces""#]);
}

#[test]
fn nothing_is_sent_unless_an_endpoint_or_file_is_named_and_a_log_less_machine_sends_nothing() {
    let collector = Collector::new(0, false, "");
    let machine = machine_with(&collector);
    let refused = export(&ExportOptions::default(), &machine.runtime).expect_err("off by default");
    assert_eq!(refused.code, "invalid-input");
    let empty = export(&ExportOptions { endpoint: Some("http://x/".into()), ..ExportOptions::default() }, &machine.runtime).expect("nothing to send");
    assert_eq!(crate::json::compact(&empty), r#"{"status":"ok","spans":0,"sent_to":[]}"#);
    assert!(collector.seen.lock().expect("seen").is_empty());
}

#[test]
fn a_collector_that_refuses_or_stalls_and_a_file_that_cannot_be_written_are_reported() {
    for (collector, expected) in [(Collector::new(22, false, "HTTP 500\n"), "refused the request: HTTP 500"), (Collector::new(28, true, ""), "did not answer in time")] {
        let machine = machine_with(&collector);
        runs(&machine);
        let error = export(&ExportOptions { endpoint: Some("http://x/".into()), ..ExportOptions::default() }, &machine.runtime).expect_err("fails");
        assert!(error.code == "export-failed" && error.message.contains(expected), "{error:?}");
    }
    let machine = machine_with(&Collector::new(0, false, ""));
    runs(&machine);
    // A folder is no file to append to.
    std::fs::create_dir_all(format!("{}/home", machine.sandbox.text())).expect("dir");
    let error = export(&ExportOptions { file: Some(format!("{}/home", machine.sandbox.text())), ..ExportOptions::default() }, &machine.runtime).expect_err("a folder");
    assert_eq!(error.code, "export-failed");
}

#[test]
fn a_run_with_no_events_has_no_span() {
    let machine = machine_with(&Collector::new(0, false, ""));
    let dir = format!("{}/state/agent-kit/model-gateway/runs/empty", machine.sandbox.text());
    std::fs::create_dir_all(&dir).expect("dir");
    let run = crate::run_index::IndexedRun { run_id: "empty".into(), run_dir: dir, workflow: "w".into(), client: "pi".into(), model: "m".into(), family: "f".into(), mode: "read-only".into(), started_ms: 0, gateway_pid: None, finished: None };
    assert!(crate::otlp::span_of(&run, &[]).is_none());
}

#[test]
fn the_arguments_set_the_options_and_the_command_prints_a_summary_or_the_refusal() {
    let options = parse_args(&strings(&["--endpoint", "e", "--file", "f", "--run", "r", "--workflow", "w"])).expect("options");
    assert_eq!(options, ExportOptions { endpoint: Some("e".into()), file: Some("f".into()), run: Some("r".into()), workflow: Some("w".into()) });
    for bad in [&["--nope", "x"][..], &["--file"]] {
        assert!(parse_args(&strings(bad)).is_none(), "{bad:?}");
    }
    let machine = machine_with(&Collector::new(0, false, ""));
    runs(&machine);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(run(&strings(&[]), &machine.runtime, &mut out, &mut err), 1);
    assert!(String::from_utf8(err.clone()).expect("utf8").contains("name an endpoint"));
    err.clear();
    assert_eq!(run(&strings(&["--bad"]), &machine.runtime, &mut out, &mut err), 1);
    let target = format!("{}/o.jsonl", machine.sandbox.text());
    assert_eq!(run(&strings(&["--file", &target]), &machine.runtime, &mut out, &mut err), 0);
    assert!(String::from_utf8(out).expect("utf8").contains("\"spans\": 4"));
}


/// The real files, except that making a scratch folder or writing a private file can be made to fail.
struct Failing {
    inner: crate::adapters::fs::SystemFiles,
    temp_dir: bool,
    write_private: bool,
}

impl crate::ports::Files for Failing {
    fn read_text(&self, path: &str) -> Result<String, crate::ports::FsError> {
        self.inner.read_text(path)
    }
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), crate::ports::FsError> {
        self.inner.write_atomic(path, content)
    }
    fn mkdir_all(&self, path: &str) -> Result<(), crate::ports::FsError> {
        self.inner.mkdir_all(path)
    }
    fn create_exclusive(&self, path: &str) -> Result<bool, crate::ports::FsError> {
        self.inner.create_exclusive(path)
    }
    fn remove_file(&self, path: &str) -> Result<(), crate::ports::FsError> {
        self.inner.remove_file(path)
    }
    fn modified_ms(&self, path: &str) -> Option<i64> {
        self.inner.modified_ms(path)
    }
    fn exists(&self, path: &str) -> Result<bool, crate::ports::FsError> {
        self.inner.exists(path)
    }
    fn append(&self, path: &str, content: &str) -> Result<(), crate::ports::FsError> {
        self.inner.append(path, content)
    }
    fn stamp(&self, path: &str) -> Option<crate::ports::FileStamp> {
        self.inner.stamp(path)
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), crate::ports::FsError> {
        if self.write_private {
            return Err(crate::ports::FsError { code: "EACCES".into(), message: "EACCES: no private file".into() });
        }
        self.inner.write_private(path, content)
    }
    fn write_text(&self, path: &str, content: &str) -> Result<(), crate::ports::FsError> {
        self.inner.write_text(path, content)
    }
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, crate::ports::FsError> {
        if self.temp_dir {
            return Err(crate::ports::FsError { code: "ENOSPC".into(), message: "ENOSPC: no scratch folder".into() });
        }
        self.inner.temp_dir(parent, prefix)
    }
    fn remove(&self, path: &str) -> Result<(), crate::ports::FsError> {
        self.inner.remove(path)
    }
    fn list_dir(&self, path: &str) -> Result<Vec<String>, crate::ports::FsError> {
        self.inner.list_dir(path)
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), crate::ports::FsError> {
        self.inner.rename(from, to)
    }
}

#[test]
fn a_scratch_folder_a_private_file_a_missing_curl_and_a_machine_with_no_state_folder_are_reported() {
    let mut machine = machine_with(&Collector::new(0, false, ""));
    machine.runtime.env.insert("OTEL_EXPORTER_OTLP_ENDPOINT".into(), "http://x".into());
    runs(&machine);
    let cwd = machine.sandbox.text();
    for (temp_dir, write_private, expected) in [(true, false, "no scratch folder"), (false, true, "no private file")] {
        machine.runtime.files = Box::new(Failing { inner: crate::adapters::fs::SystemFiles { cwd: cwd.clone() }, temp_dir, write_private });
        let error = export(&ExportOptions::default(), &machine.runtime).expect_err("fails");
        assert!(error.code == "export-failed" && error.message.contains(expected), "{error:?}");
    }
    machine.runtime.files = Box::new(crate::adapters::fs::SystemFiles { cwd });
    let mut gone = Collector::new(0, false, "");
    gone.missing = true;
    machine.runtime.process = Box::new(gone);
    let error = export(&ExportOptions::default(), &machine.runtime).expect_err("no curl");
    assert!(error.message.contains("curl is not installed"), "{error:?}");
    machine.runtime.env.remove("HOME");
    machine.runtime.env.remove("XDG_STATE_HOME");
    assert!(export(&ExportOptions::default(), &machine.runtime).is_err());
}

#[test]
fn the_command_line_routes_export_here_and_a_bare_one_exports_nothing() {
    let machine = machine_with(&Collector::new(0, false, ""));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(crate::run_with(["export".to_string()].into_iter(), &machine.runtime, &mut out, &mut err), 1);
    assert!(String::from_utf8(err).expect("utf8").contains("export takes"));
    assert!(out.is_empty());
}
