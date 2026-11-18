//! The routing record: what LiteLLM's response headers say of a run, read through the recording proxy. The stub
//! servers here are real sockets on 127.0.0.1 and the forwarding is the real `curl`, so a forced fallback shows
//! up as `attempted_fallbacks >= 1` the way it would from the live proxy.

use super::machine::{Machine, DEFAULTS};
use super::scenarios::EXEC_LOCK;
use crate::json::{parse, Value};
use crate::ports::{Process, ProcessCommand, ProcessResult};
use crate::routing::{recorded as record_with, seen_of, served_model_of, split_base, Capture, Routing, Seen};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

fn header_list(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
}

#[test]
fn the_model_that_answered_is_the_first_model_string_in_the_body_or_the_first_event() {
    assert_eq!(served_model_of(r#"{"id":"c1","model":"free/glm-4.7-flash","choices":[]}"#).as_deref(), Some("free/glm-4.7-flash"));
    assert_eq!(served_model_of("data: {\"id\":1,\"model\" : \"m-served\"}\n\ndata: [DONE]\n\n").as_deref(), Some("m-served"));
    assert_eq!(served_model_of(r#"{"model":"a\"b"}"#).as_deref(), Some("a\"b"));
    for none in [r#"{"id":"c1"}"#, r#"{"model":""}"#, r#"{"model" "x"}"#, r#"{"model": 3}"#, r#"{"model":"never ends"#, r#"{"model":"ends with a slash\"#, ""] {
        assert_eq!(served_model_of(none), None, "{none}");
    }
}

#[test]
fn the_routing_headers_are_read_in_any_case_and_a_bad_count_is_unknown() {
    let seen = seen_of(&header_list(&[("X-LiteLLM-Model-Id", "free/glm/openrouter"), ("x-litellm-attempted-fallbacks", "2")]), r#"{"model":"glm"}"#);
    assert_eq!(seen, Seen { model_id: Some("free/glm/openrouter".into()), attempted_fallbacks: Some(2), served_model: Some("glm".into()) });
    let bare = seen_of(&header_list(&[("x-litellm-model-id", ""), ("x-litellm-attempted-fallbacks", "many")]), "");
    assert_eq!(bare, Seen::default());
}

#[test]
fn a_runs_routing_is_the_last_serving_model_and_the_most_fallbacks_any_request_needed() {
    let one = |id: &str, fallbacks: Option<u64>, served: &str| Seen { model_id: Some(id.into()), attempted_fallbacks: fallbacks, served_model: Some(served.into()) };
    let routing = Routing::of(&[one("a", Some(1), "m1"), one("b", None, "m2")]);
    assert_eq!((routing.capture, routing.model_id.as_deref(), routing.served_model.as_deref(), routing.attempted_fallbacks), (Capture::Recorded, Some("b"), Some("m2"), Some(1)));
    assert_eq!(routing.fallback_fired(), Some(true));
    // A response with LiteLLM's id and no fallbacks header needed none.
    let plain = Routing::of(&[one("a", None, "m1")]);
    assert_eq!((plain.attempted_fallbacks, plain.fallback_fired()), (Some(0), Some(false)));
    // A response with none of LiteLLM's headers did not come from it: the fallbacks are not known.
    let foreign = Routing::of(&[Seen::default()]);
    assert_eq!((foreign.capture, foreign.attempted_fallbacks, foreign.fallback_fired()), (Capture::Recorded, None, None));
    let none = Routing::of(&[]);
    assert_eq!((none.capture, none.model_id, none.attempted_fallbacks), (Capture::NoRequest, None, None));
}

#[test]
fn a_client_the_gateway_does_not_stand_in_front_of_records_why_nothing_was_seen() {
    for client in ["claude", "codex", "agy", "opencode-free"] {
        assert_eq!(Routing::unseen(client).capture, Capture::NotLitellm, "{client}");
    }
    for client in ["pi-free", "vibe", "hermes", "opencode"] {
        assert_eq!(Routing::unseen(client).capture, Capture::Hidden, "{client}");
    }
    let names: Vec<&str> = [Capture::Recorded, Capture::NoRequest, Capture::Hidden, Capture::NotLitellm].iter().map(|capture| capture.as_str()).collect();
    assert_eq!(names, ["recorded", "no-request", "hidden", "not-litellm"]);
    let unseen = Routing::unseen("pi-free");
    assert_eq!((unseen.fallback_fired(), unseen.model_id, unseen.served_model, unseen.attempted_fallbacks), (None, None, None, None));
}

#[test]
fn a_base_url_splits_into_the_origin_the_proxy_forwards_to_and_the_path_the_client_adds_to() {
    assert_eq!(split_base("https://litellm.example/v1"), Some(("https://litellm.example".to_string(), "/v1".to_string())));
    assert_eq!(split_base("http://127.0.0.1:4000"), Some(("http://127.0.0.1:4000".to_string(), String::new())));
    assert_eq!(split_base("litellm.example/v1"), None);
}

// ---- a stub LiteLLM and a client of the proxy, as real sockets ----

struct Stub {
    port: u16,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Stub {
    fn requests(&self) -> Vec<String> {
        self.seen.lock().expect("requests").clone()
    }
}

/// The whole request on a connection, head and body, as text.
fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut wanted: Option<usize> = None;
    loop {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if let Some(end) = text.find("\r\n\r\n") {
            let length = *wanted.get_or_insert_with(|| {
                text.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|value| value.trim().parse().ok())).unwrap_or(0)
            });
            if bytes.len() >= end + 4 + length {
                return text;
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return text,
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
        }
    }
}

/// A server that answers its connections with `responses`, one each, in order, and notes what it was sent.
fn stub(responses: Vec<Vec<u8>>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let text = read_request(&mut stream);
            log.lock().expect("log").push(text);
            let _ = stream.write_all(&response);
        }
    });
    Stub { port, seen }
}

/// A `200` from LiteLLM with these headers and body.
fn answer(headers: &[(&str, &str)], body: &str) -> Vec<u8> {
    let extra: String = headers.iter().map(|(key, value)| format!("{key}: {value}\r\n")).collect();
    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes()
}

/// What a client sends the proxy: a POST of `body` with the key as Pi sends it.
fn post(port: u16, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let request = format!("POST {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer literal-key\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    stream.write_all(request.as_bytes()).expect("send");
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// Sends raw bytes, closes the sending side, and reads what comes back.
fn raw(port: u16, bytes: &[u8]) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let _ = stream.write_all(bytes);
    let _ = stream.shutdown(Shutdown::Write);
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// The port the per-run home points the client at.
fn proxy_port(env: &HashMap<String, String>) -> u16 {
    let text = std::fs::read_to_string(format!("{}/models.json", env["PI_CODING_AGENT_DIR"])).expect("the per-run models.json");
    let url = parse(&text).expect("JSON").as_object().and_then(|o| o.get("providers")).and_then(Value::as_object).and_then(|p| p.get("litellm-free")).and_then(Value::as_object).and_then(|e| e.get("baseUrl")).and_then(Value::as_str).map(str::to_string).expect("a baseUrl");
    url.strip_prefix("http://127.0.0.1:").and_then(|rest| rest.split('/').next()).and_then(|port| port.parse().ok()).expect("a local port")
}

/// A machine whose Pi home has the free route's provider at `base_url`.
fn machine_with(base_url: &str) -> Machine {
    let machine = Machine::new(DEFAULTS);
    let provider = format!(
        r#"{{"providers":{{"litellm-free":{{"api":"openai-completions","apiKey":"literal-key","baseUrl":"{base_url}","models":[{{"id":"tier-free","cost":{{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}}}}]}}}}}}"#
    );
    machine.write_at(&format!("{}/home/.pi/agent/models.json", machine.sandbox.text()), &provider);
    machine
}

/// `recorded` for a launch that gives a value back, with the free route's provider.
fn recorded<R>(runtime: &crate::ports::Runtime, curl: &str, dir: &str, provider: &str, client: &str, launch: impl FnOnce(&HashMap<String, String>) -> R) -> (R, Routing) {
    let mut launch = Some(launch);
    let mut out = None;
    let routing = record_with(runtime, curl, dir, provider, client, &mut |env| out = launch.take().map(|launch| launch(env)));
    (out.expect("the launch ran"), routing)
}

fn home_of(machine: &Machine) -> String {
    format!("{}/proxy-home", machine.sandbox.text())
}

#[test]
fn a_forced_fallback_shows_attempted_fallbacks_and_the_model_that_served_the_run() {
    let upstream = stub(vec![
        answer(&[("x-litellm-model-id", "free/glm-4.7-flash/openrouter"), ("x-litellm-attempted-fallbacks", "1")], r#"{"id":"c1","model":"free/glm-4.7-flash","choices":[]}"#),
        answer(&[("X-LiteLLM-Model-Id", "free/qwen3.8-27b/nvidia")], r#"{"id":"c2","model":"free/qwen3.8-27b","choices":[]}"#),
    ]);
    let machine = machine_with(&format!("http://127.0.0.1:{}/v1", upstream.port));
    let home = home_of(&machine);
    let (answers, routing) = recorded(&machine.runtime, "curl", &home, "litellm-free", "pi-free", |env| {
        let port = proxy_port(env);
        (post(port, "/v1/chat/completions", r#"{"model":"tier-free"}"#), post(port, "/v1/chat/completions", r#"{"model":"tier-free"}"#))
    });
    assert!(answers.0.starts_with("HTTP/1.1 200 OK\r\n") && answers.0.contains("Connection: close") && answers.0.ends_with(r#""choices":[]}"#), "{}", answers.0);
    assert!(answers.0.to_ascii_lowercase().contains("x-litellm-attempted-fallbacks: 1"), "the headers are passed back as they came");
    assert_eq!(routing.capture, Capture::Recorded);
    assert!(routing.attempted_fallbacks >= Some(1), "{routing:?}");
    assert_eq!(routing.fallback_fired(), Some(true));
    assert_eq!((routing.model_id.as_deref(), routing.served_model.as_deref()), (Some("free/qwen3.8-27b/nvidia"), Some("free/qwen3.8-27b")));
    // The real LiteLLM was sent the tier's name as the model, and the client's key, as the client sent them.
    let sent = upstream.requests();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].starts_with("POST /v1/chat/completions HTTP/1.1\r\n") && sent[0].ends_with(r#"{"model":"tier-free"}"#), "{}", sent[0]);
    assert!(sent[0].to_ascii_lowercase().contains("authorization: bearer literal-key"), "{}", sent[0]);
    // The per-run home, which held the route's address and its curl configs, is gone.
    assert!(!Path::new(&home).exists());
}

#[test]
fn a_client_that_makes_no_request_is_recorded_as_none_and_its_home_is_copied_with_its_bin() {
    let machine = machine_with("https://litellm.example/v1");
    machine.write_at(&format!("{}/home/.pi/agent/bin/fd", machine.sandbox.text()), "fd");
    let home = home_of(&machine);
    let (seen, routing) = recorded(&machine.runtime, "curl", &home, "litellm-free", "pi-free", |env| {
        let dir = env["PI_CODING_AGENT_DIR"].clone();
        let models = std::fs::read_to_string(format!("{dir}/models.json")).expect("models.json");
        (models, Path::new(&format!("{dir}/bin/fd")).exists())
    });
    assert!(seen.0.contains("\"baseUrl\": \"http://127.0.0.1:") && seen.0.contains("/v1\"") && seen.0.contains("\"apiKey\": \"literal-key\""), "{}", seen.0);
    assert!(seen.1, "the home's bin is beside the per-run models.json");
    assert_eq!(routing.capture, Capture::NoRequest);
    assert!(!Path::new(&home).exists());
}

#[test]
fn a_run_the_proxy_cannot_stand_in_front_of_runs_on_the_gateways_own_home_and_says_hidden() {
    let cases: [(&str, Option<&str>); 5] = [
        ("no models.json", None),
        ("not JSON", Some("{")),
        ("no such provider", Some(r#"{"providers":{"litellm-vibe":{"baseUrl":"https://x/v1"}}}"#)),
        ("no base URL", Some(r#"{"providers":{"litellm-free":{"api":"x"}}}"#)),
        ("a base URL that is none", Some(r#"{"providers":{"litellm-free":{"baseUrl":"nope"}}}"#)),
    ];
    for (what, models) in cases {
        let machine = Machine::new(DEFAULTS);
        if let Some(models) = models {
            machine.write_at(&format!("{}/home/.pi/agent/models.json", machine.sandbox.text()), models);
        }
        let home = home_of(&machine);
        let (own, routing) = recorded(&machine.runtime, "curl", &home, "litellm-free", "pi-free", |env| env.get("PI_CODING_AGENT_DIR").cloned());
        assert_eq!((own, routing.capture), (None, Capture::Hidden), "{what}");
        assert!(!Path::new(&home).exists(), "{what}");
    }
}

#[test]
fn a_request_the_proxy_cannot_read_is_refused_with_a_status_that_says_why() {
    let machine = machine_with("https://litellm.example/v1");
    let home = home_of(&machine);
    let (said, routing) = recorded(&machine.runtime, "curl", &home, "litellm-free", "pi-free", |env| {
        let port = proxy_port(env);
        let head_only = format!("GET / HTTP/1.1\r\nX: {}", "a".repeat(64 * 1024 + 1 - "GET / HTTP/1.1\r\nX: ".len()));
        [
            raw(port, b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"),
            raw(port, head_only.as_bytes()),
            raw(port, b"GET / HTTP/1.1\r\n"),
            raw(port, b"\r\n\r\n"),
            raw(port, b"POST /x HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc"),
        ]
    });
    let statuses: Vec<&str> = said.iter().map(|text| text.lines().next().unwrap_or("")).collect();
    assert_eq!(
        statuses,
        ["HTTP/1.1 411 Bad Gateway", "HTTP/1.1 431 Bad Gateway", "HTTP/1.1 400 Bad Gateway", "HTTP/1.1 400 Bad Gateway", "HTTP/1.1 400 Bad Gateway"]
    );
    assert!(said[0].contains("a chunked request body is not supported") && said[4].contains("the request body ended early"), "{said:?}");
    assert_eq!(routing.capture, Capture::NoRequest);
}

#[test]
fn a_body_that_arrives_after_its_head_is_read_whole_and_forwarded() {
    let upstream = stub(vec![answer(&[("x-litellm-model-id", "m")], r#"{"model":"served"}"#)]);
    let machine = machine_with(&format!("http://127.0.0.1:{}/v1", upstream.port));
    let home = home_of(&machine);
    let (answered, _) = recorded(&machine.runtime, "curl", &home, "litellm-free", "pi-free", |env| {
        let mut stream = TcpStream::connect(("127.0.0.1", proxy_port(env))).expect("connect");
        stream.write_all(b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 10\r\n\r\n01234").expect("head and part of the body");
        std::thread::sleep(std::time::Duration::from_millis(100));
        stream.write_all(b"56789").expect("the rest");
        let mut out = Vec::new();
        let _ = stream.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    });
    assert!(answered.starts_with("HTTP/1.1 200 OK"), "{answered}");
    assert!(upstream.requests()[0].ends_with("0123456789"), "{:?}", upstream.requests());
}

#[test]
fn a_request_that_cannot_be_forwarded_is_a_502_and_is_not_recorded() {
    // Nothing listens on port 1, so curl cannot connect.
    let machine = machine_with("http://127.0.0.1:1/v1");
    let home = home_of(&machine);
    let (unreachable, routing) = recorded(&machine.runtime, "curl", &home, "litellm-free", "pi-free", |env| post(proxy_port(env), "/v1/chat/completions", "{}"));
    assert!(unreachable.starts_with("HTTP/1.1 502 ") && unreachable.contains("the proxy could not reach LiteLLM"), "{unreachable}");
    assert_eq!(routing.capture, Capture::NoRequest);
    let (no_curl, _) = recorded(&machine.runtime, "/nonexistent/curl", &home, "litellm-free", "pi-free", |env| post(proxy_port(env), "/x", "{}"));
    assert!(no_curl.starts_with("HTTP/1.1 502 ") && no_curl.contains("the proxy cannot start /nonexistent/curl"), "{no_curl}");
    // A home that went away under the proxy cannot hold its curl config.
    let (no_config, _) = recorded(&machine.runtime, "curl", &home, "litellm-free", "pi-free", |env| {
        let port = proxy_port(env);
        std::fs::remove_dir_all(&env["PI_CODING_AGENT_DIR"]).expect("remove the home");
        post(port, "/x", "{}")
    });
    assert!(no_config.starts_with("HTTP/1.1 502 ") && no_config.contains("cannot write its curl config"), "{no_config}");
}

/// A `curl` that prints `output` whatever it is asked, for the shapes a stub server cannot make curl print.
fn fake_curl(machine: &Machine, output: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = format!("{}/fake-curl", machine.sandbox.text());
    std::fs::write(&path, format!("#!/bin/sh\ncat > /dev/null\nprintf '%b' '{output}'\n")).expect("write the fake curl");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

#[test]
fn an_interim_response_is_skipped_a_stream_is_relayed_and_a_get_has_no_body() {
    let _held = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let machine = machine_with("https://litellm.example/v1");
    let home = home_of(&machine);
    let output = "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-LiteLLM-Attempted-Fallbacks: 0\r\nX-LiteLLM-Model-Id: m1\r\nTransfer-Encoding: chunked\r\n\r\ndata: {\"model\":\"m-served\"}\n\ndata: [DONE]\n\n";
    let curl = fake_curl(&machine, &output.replace('\r', "\\r").replace('\n', "\\n"));
    let (streamed, routing) = recorded(&machine.runtime, &curl, &home, "litellm-free", "pi-free", |env| raw(proxy_port(env), b"GET /v1/models HTTP/1.1\r\nHost: x\r\n\r\n"));
    assert!(streamed.starts_with("HTTP/1.1 200 OK\r\n") && streamed.contains("Connection: close") && !streamed.contains("Transfer-Encoding"), "{streamed}");
    assert!(streamed.ends_with("data: [DONE]\n\n"), "{streamed}");
    assert_eq!((routing.capture, routing.model_id.as_deref(), routing.served_model.as_deref(), routing.attempted_fallbacks), (Capture::Recorded, Some("m1"), Some("m-served"), Some(0)));
    assert_eq!(routing.fallback_fired(), Some(false));
}

#[test]
fn a_client_that_goes_away_mid_answer_still_has_the_headers_it_got_recorded() {
    // The stub answers with far more than the sockets hold, so the proxy's write to the gone client fails.
    let big = "x".repeat(8 * 1024 * 1024);
    let upstream = stub(vec![answer(&[("x-litellm-model-id", "free/big"), ("x-litellm-attempted-fallbacks", "3")], &format!("{{\"model\":\"m-big\",\"pad\":\"{big}\"}}"))]);
    let machine = machine_with(&format!("http://127.0.0.1:{}/v1", upstream.port));
    let home = home_of(&machine);
    let ((), routing) = recorded(&machine.runtime, "curl", &home, "litellm-free", "pi-free", |env| {
        let mut stream = TcpStream::connect(("127.0.0.1", proxy_port(env))).expect("connect");
        stream.write_all(b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}").expect("send");
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|count| count == 1) {
            head.push(byte[0]);
        }
        assert!(head.starts_with(b"HTTP/1.1 200 OK"));
    });
    assert_eq!((routing.capture, routing.model_id.as_deref(), routing.attempted_fallbacks), (Capture::Recorded, Some("free/big"), Some(3)));
}

// ---- a whole tier run, through the CLI ----

/// Pi as the launcher would start it, for a stub: it sends the proxy the requests Pi would, with the model it was named.
struct StubPi {
    asked: Arc<Mutex<Vec<String>>>,
}

impl Process for StubPi {
    fn resolve(&self, binary: &str) -> Option<String> {
        Some(format!("/bin/{binary}"))
    }

    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Ok(ProcessResult { stdout: "signed in with an api key\n".into(), stderr: String::new(), exit_code: 0, timed_out: false })
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        let after = |flag: &str| command.args.iter().position(|arg| arg == flag).and_then(|at| command.args.get(at + 1)).cloned().expect("the flag");
        let port: u16 = after("--net-connect").parse().expect("a port");
        let model = after("--model");
        let body = format!("{{\"model\":\"{model}\",\"messages\":[]}}");
        for _ in 0..2 {
            let answer = post(port, "/v1/chat/completions", &body);
            self.asked.lock().expect("asked").push(answer.lines().next().unwrap_or("").to_string());
        }
        let message = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Looked.\nSUMMARY: pi is fine"}],"stopReason":"stop","usage":{"input":100,"output":40,"cacheRead":30,"cost":{"total":0}}}}"#;
        Ok(ProcessResult { stdout: format!("{message}\n"), stderr: String::new(), exit_code: 0, timed_out: false })
    }
}

#[test]
fn a_tier_run_sends_the_tier_as_the_model_and_writes_one_ledger_line_with_the_routing() {
    let upstream = stub(vec![
        answer(&[("x-litellm-model-id", "free/hy4-flash-free/opencode-go"), ("x-litellm-attempted-fallbacks", "1")], r#"{"model":"free/longcat-2.5-free","choices":[]}"#),
        answer(&[("x-litellm-model-id", "free/longcat-2.5-free/opencode-go")], r#"{"model":"free/longcat-2.5-free","choices":[]}"#),
    ]);
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut machine = Machine::with_process(crate::SHIPPED_DEFAULTS, super::machine::now_ms(), super::machine::FakeProcess::default(), Box::new(StubPi { asked: asked.clone() }));
    machine.runtime.env.insert("AGENT_KIT_LAUNCHER_BIN".to_string(), "/opt/agent-launcher".to_string());
    let provider = format!(
        r#"{{"providers":{{"litellm-free":{{"api":"openai-completions","apiKey":"literal-key","baseUrl":"http://127.0.0.1:{}/v1","models":[{{"id":"tier-free","cost":{{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}}}}]}}}}}}"#,
        upstream.port
    );
    machine.write_at(&format!("{}/home/.pi/agent/models.json", machine.sandbox.text()), &provider);
    let argv: Vec<String> = ["run", "--tier", "tier-free", "--json", r#"{"workflow":"w","mode":"read-only","prompt":"p","escalate":false}"#].iter().map(|arg| arg.to_string()).collect();
    let result = crate::cli::run_cli(&argv, &machine.runtime);
    assert_eq!(result.exit_code, 0, "{}{}", result.stdout, result.stderr);
    assert!(result.stdout.contains("\"status\": \"ok\"") && !result.stdout.contains("attempted_fallbacks"), "{}", result.stdout);
    assert_eq!(*asked.lock().expect("asked"), ["HTTP/1.1 200 OK", "HTTP/1.1 200 OK"]);
    // LiteLLM was sent model: tier-free, with the route's key.
    let sent = upstream.requests();
    assert!(sent.iter().all(|request| request.ends_with(r#"{"model":"tier-free","messages":[]}"#)), "{sent:?}");
    assert!(sent[0].to_ascii_lowercase().contains("authorization: bearer literal-key"));
    // One ledger line, with the routing record.
    let ledger = machine.ledger_text();
    assert_eq!(ledger.lines().count(), 1, "{ledger}");
    let line = parse(ledger.lines().next().expect("a line")).expect("JSON");
    let line = line.as_object().expect("an object");
    let text = |key: &str| line.get(key).and_then(Value::as_str).map(str::to_string);
    assert_eq!((text("tier").as_deref(), text("harness").as_deref(), text("client").as_deref()), (Some("tier-free"), Some("pi"), Some("pi-free")));
    assert_eq!((text("litellm_model_id").as_deref(), text("served_model").as_deref(), text("routing_capture").as_deref()), (Some("free/longcat-2.5-free/opencode-go"), Some("free/longcat-2.5-free"), Some("recorded")));
    assert_eq!((line.get("attempted_fallbacks"), line.get("fallback_fired")), (Some(&Value::Num(1.0)), Some(&Value::Bool(true))));
    assert!(matches!(line.get("cost_usd"), Some(Value::Num(_))));
}
