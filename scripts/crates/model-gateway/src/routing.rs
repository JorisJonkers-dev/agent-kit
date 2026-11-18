//! What a run's routing looked like to LiteLLM: which model served it and whether a fallback fired.
//!
//! LiteLLM says so in the headers of its responses (`x-litellm-model-id`, `x-litellm-attempted-fallbacks`) and
//! names the model that answered in the body. A delegate client talks to the proxy itself and shows the gateway
//! none of that, so for a tier run the gateway stands between them: it starts a small recording proxy on
//! `127.0.0.1`, points the client's route at it (a per-run copy of the Pi home whose provider has the proxy as its
//! base URL, so the confinement allows exactly that port), and forwards every request to the real proxy with `curl`,
//! which speaks TLS. Each response is passed back as it arrives, and its headers are noted on the way.
//!
//! The client's own key travels in the request it sends, as it did before; it is handed to `curl` in a private
//! config file, never on a command line.
//!
//! A client the gateway does not stand in front of records `hidden`, and one that never goes through LiteLLM
//! records `not-litellm`: the ledger says what it could not see, it does not guess.

use crate::json::{self, Object, Value};
use crate::pi_launch::home_agent_dir;
use crate::ports::Runtime;
use crate::text::join;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// The clients that never call LiteLLM: their routing is not LiteLLM's to report.
const NOT_LITELLM: [&str; 4] = ["claude", "codex", "agy", "opencode-free"];

/// The most of a request's head the proxy reads before it gives up on it.
const MAX_HEAD: usize = 64 * 1024;

/// How much of a response's body is searched for the model that answered.
const BODY_PREFIX: usize = 8 * 1024;

/// How the gateway knew a run's routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    /// The gateway saw LiteLLM's response headers on at least one request.
    Recorded,
    /// The gateway stood in front of the client, and the client made no request that was answered.
    NoRequest,
    /// The client talks to LiteLLM itself and hides its headers from the gateway.
    Hidden,
    /// The client does not use LiteLLM.
    NotLitellm,
}

impl Capture {
    pub fn as_str(self) -> &'static str {
        match self {
            Capture::Recorded => "recorded",
            Capture::NoRequest => "no-request",
            Capture::Hidden => "hidden",
            Capture::NotLitellm => "not-litellm",
        }
    }
}

/// What the proxy noted of one answered request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub model_id: Option<String>,
    pub attempted_fallbacks: Option<u64>,
    pub served_model: Option<String>,
}

/// A run's routing, as the ledger line writes it: `None` is a value the gateway could not see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Routing {
    pub capture: Capture,
    pub model_id: Option<String>,
    pub served_model: Option<String>,
    pub attempted_fallbacks: Option<u64>,
}

impl Routing {
    /// The routing of a run the gateway did not stand in front of: nothing seen, and why.
    pub fn unseen(client: &str) -> Routing {
        let capture = if NOT_LITELLM.contains(&client) { Capture::NotLitellm } else { Capture::Hidden };
        Routing { capture, model_id: None, served_model: None, attempted_fallbacks: None }
    }

    /// The routing of what the proxy saw: the last answered request names the serving model, and the most
    /// fallbacks any request needed is the run's.
    pub fn of(seen: &[Seen]) -> Routing {
        let Some(last) = seen.last() else {
            return Routing { capture: Capture::NoRequest, model_id: None, served_model: None, attempted_fallbacks: None };
        };
        // A response that carries none of LiteLLM's headers did not come from it, so its fallbacks are not known; one
        // that does, and says nothing of fallbacks, needed none.
        let answered = seen.iter().any(|one| one.model_id.is_some() || one.attempted_fallbacks.is_some());
        Routing {
            capture: Capture::Recorded,
            model_id: seen.iter().rev().find_map(|one| one.model_id.clone()),
            served_model: last.served_model.clone(),
            attempted_fallbacks: answered.then(|| seen.iter().filter_map(|one| one.attempted_fallbacks).max().unwrap_or(0)),
        }
    }

    /// Whether LiteLLM fell back to another deployment, or `None` when the gateway could not tell.
    pub fn fallback_fired(&self) -> Option<bool> {
        self.attempted_fallbacks.map(|count| count >= 1)
    }
}

/// The model a response's body names: the first `"model"` string in what the proxy has of it, in JSON or in the
/// first event of a stream.
pub fn served_model_of(body: &str) -> Option<String> {
    let at = body.find("\"model\"")? + "\"model\"".len();
    let rest = body[at..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut name = String::new();
    let mut chars = rest.chars();
    loop {
        match chars.next()? {
            '"' => return (!name.is_empty()).then_some(name),
            '\\' => name.push(chars.next()?),
            other => name.push(other),
        }
    }
}

/// The value of a header, by name in any case.
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
}

/// What a response's headers and the start of its body say of the routing.
pub fn seen_of(headers: &[(String, String)], body_prefix: &str) -> Seen {
    Seen {
        model_id: header(headers, "x-litellm-model-id").filter(|id| !id.is_empty()).map(str::to_string),
        attempted_fallbacks: header(headers, "x-litellm-attempted-fallbacks").and_then(|count| count.parse().ok()),
        served_model: served_model_of(body_prefix),
    }
}

/// The index of the end of a head (the blank line), as the first byte after it.
fn head_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n").map(|at| at + 4)
}

/// The header lines of a head, split at the first colon.
fn header_lines(head: &str) -> Vec<(String, String)> {
    head.split("\r\n").skip(1).filter_map(|line| line.split_once(':')).map(|(key, value)| (key.trim().to_string(), value.trim().to_string())).collect()
}

/// A request as the client sent it to the proxy.
struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Why a request was not forwarded: the status the client is told, and the words.
struct Refusal(u16, String);

/// Reads one request off the client's connection.
fn read_request(stream: &mut TcpStream) -> Result<Request, Refusal> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let end = loop {
        if let Some(end) = head_end(&buffer) {
            break end;
        }
        if buffer.len() > MAX_HEAD {
            return Err(Refusal(431, "the request head is too large".into()));
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(Refusal(400, "the request ended before its head did".into())),
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
        }
    };
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let mut first = head.split("\r\n").next().unwrap_or("").split(' ');
    let (Some(method), Some(target)) = (first.next(), first.next()) else {
        return Err(Refusal(400, "the request line is not a method and a target".into()));
    };
    let headers = header_lines(&head);
    if header(&headers, "transfer-encoding").is_some() {
        return Err(Refusal(411, "a chunked request body is not supported: send a content-length".into()));
    }
    let length = header(&headers, "content-length").and_then(|value| value.parse::<usize>().ok()).unwrap_or(0);
    let mut body = buffer[end..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(Refusal(400, "the request body ended early".into())),
            Ok(count) => body.extend_from_slice(&chunk[..count]),
        }
    }
    body.truncate(length);
    Ok(Request { method: method.to_string(), target: target.to_string(), headers, body })
}

/// The headers a forwarded request does not carry: the connection's own, and the ones `curl` sets itself.
const NOT_FORWARDED: [&str; 6] = ["host", "content-length", "connection", "transfer-encoding", "expect", "keep-alive"];

/// The headers a relayed response does not carry: the proxy ends the body by closing the connection.
const NOT_RELAYED: [&str; 4] = ["content-length", "connection", "transfer-encoding", "keep-alive"];

static CONFIGS: AtomicUsize = AtomicUsize::new(0);

/// What one proxy needs to forward: where the real proxy is, where its private files go, and what program speaks to it.
struct Upstream<'a> {
    origin: &'a str,
    dir: &'a str,
    curl: &'a str,
}

/// A curl config holding the forwarded headers, so a key is in a file only its owner reads and not on a command line;
/// `None` when the file cannot be made.
fn curl_config(upstream: &Upstream, request: &Request) -> Option<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = format!("{}/curl-{}.conf", upstream.dir, CONFIGS.fetch_add(1, Ordering::Relaxed));
    let mut text = String::new();
    for (key, value) in request.headers.iter().filter(|(key, _)| !NOT_FORWARDED.contains(&key.to_ascii_lowercase().as_str())) {
        let line = format!("{key}: {value}").replace('\\', "\\\\").replace('"', "\\\"");
        text.push_str(&format!("header = \"{line}\"\n"));
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).ok()?;
    file.write_all(text.as_bytes()).ok()?;
    Some(path)
}

/// Forwards one request and relays the answer, noting the routing headers on the way. A request that cannot be
/// forwarded is answered with the status that says so.
fn forward(stream: &mut TcpStream, request: &Request, upstream: &Upstream, seen: &Mutex<Vec<Seen>>) -> Result<(), Refusal> {
    let config = curl_config(upstream, request).ok_or_else(|| Refusal(502, "the proxy cannot write its curl config".to_string()))?;
    let url = format!("{}{}", upstream.origin, request.target);
    let spawned = Command::new(upstream.curl)
        .args(["-sS", "-i", "--no-buffer", "--http1.1", "--config", &config, "-X", &request.method, "--data-binary", "@-", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = spawned.map_err(|error| Refusal(502, format!("the proxy cannot start {}: {error}", upstream.curl)))?;
    let _ = child.stdin.take().expect("stdin was piped").write_all(&request.body);
    let mut out = child.stdout.take().expect("stdout was piped");
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    // The answer's head: a `100 Continue` before it is skipped.
    let (status_line, headers, rest) = loop {
        let Some(end) = head_end(&buffer) else {
            match out.read(&mut chunk) {
                Ok(0) | Err(_) => {
                    let said = child.wait_with_output().map(|done| String::from_utf8_lossy(&done.stderr).trim().to_string()).unwrap_or_default();
                    let _ = std::fs::remove_file(&config);
                    return Err(Refusal(502, format!("the proxy could not reach LiteLLM: {said}")));
                }
                Ok(count) => {
                    buffer.extend_from_slice(&chunk[..count]);
                    continue;
                }
            }
        };
        let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
        let status_line = head.split("\r\n").next().unwrap_or("").to_string();
        if status_line.split(' ').nth(1).is_some_and(|code| code.starts_with('1')) {
            buffer.drain(..end);
            continue;
        }
        break (status_line, header_lines(&head), buffer[end..].to_vec());
    };
    let _ = std::fs::remove_file(&config);
    let code = status_line.split(' ').nth(1).unwrap_or("502");
    let reason = status_line.splitn(3, ' ').nth(2).unwrap_or("");
    let mut head = format!("HTTP/1.1 {code} {reason}\r\n");
    for (key, value) in headers.iter().filter(|(key, _)| !NOT_RELAYED.contains(&key.to_ascii_lowercase().as_str())) {
        head.push_str(&format!("{key}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    let mut relayed = stream.write_all(head.as_bytes()).is_ok();
    let mut prefix: Vec<u8> = rest.clone();
    relayed = relayed && stream.write_all(&rest).is_ok();
    while relayed {
        match out.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if prefix.len() < BODY_PREFIX {
                    prefix.extend_from_slice(&chunk[..count]);
                }
                relayed = stream.write_all(&chunk[..count]).is_ok();
            }
        }
    }
    if !relayed {
        let _ = child.kill();
    }
    let _ = child.wait();
    let noted = seen_of(&headers, &String::from_utf8_lossy(&prefix[..prefix.len().min(BODY_PREFIX)]));
    seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(noted);
    Ok(())
}

/// Answers one connection: one request, forwarded, then the connection closes.
fn serve_one(mut stream: TcpStream, upstream: &Upstream, seen: &Mutex<Vec<Seen>>) {
    let _ = stream.set_nonblocking(false);
    let outcome = read_request(&mut stream).and_then(|request| forward(&mut stream, &request, upstream, seen));
    if let Err(Refusal(status, message)) = outcome {
        let body = json::compact(&json::object(vec![("error", Value::str(&message))]));
        let _ = write!(stream, "HTTP/1.1 {status} Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    }
}

/// Serves connections until told to stop, each on a thread of its own.
fn serve(listener: &TcpListener, stop: &AtomicBool, upstream: &Upstream, seen: &Mutex<Vec<Seen>>) {
    std::thread::scope(|scope| {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    scope.spawn(move || serve_one(stream, upstream, seen));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    });
}

/// Where a provider's real proxy is, from the home's `models.json`: the origin (`https://host`), and the path
/// its base URL carries (`/v1`).
pub fn split_base(base_url: &str) -> Option<(String, String)> {
    let scheme = base_url.find("://")? + 3;
    let slash = base_url[scheme..].find('/').map_or(base_url.len(), |at| scheme + at);
    Some((base_url[..slash].to_string(), base_url[slash..].to_string()))
}

/// The per-run Pi home that points the provider at the proxy: its `models.json` is the provider's entry with the
/// proxy as its base URL, and the home's `bin` is copied beside it. Returns the real proxy's origin, or `None` when
/// the home names no such provider or the per-run home cannot be written.
fn proxy_home(runtime: &Runtime, dir: &str, provider: &str, port: u16) -> Option<String> {
    let home = home_agent_dir(&runtime.env);
    let parsed = json::parse(&runtime.files.read_text(&join(&[&home, "models.json"])).ok()?).ok()?;
    let entry = parsed.as_object()?.get("providers")?.as_object()?.get(provider)?.as_object()?;
    let (origin, path) = split_base(entry.get("baseUrl")?.as_str()?)?;
    let mut recorded: Object = entry.clone();
    recorded.set("baseUrl", Value::Str(format!("http://127.0.0.1:{port}{path}")));
    let models = json::object(vec![("providers", json::object(vec![(provider, Value::Obj(recorded))]))]);
    runtime.files.write_private(&join(&[dir, "models.json"]), &format!("{}\n", json::pretty(&models))).ok()?;
    let bin = join(&[&home, "bin"]);
    if runtime.files.exists(&bin).unwrap_or(false) {
        runtime.files.copy_tree(&bin, &join(&[dir, "bin"])).ok()?;
    }
    Some(origin)
}

/// The launch with the gateway's own environment, when no proxy stands in front of it: its routing is unseen.
fn unproxied(runtime: &Runtime, dir: &str, client: &str, launch: &mut dyn FnMut(&HashMap<String, String>)) -> Routing {
    let _ = runtime.files.remove(dir);
    launch(&runtime.env);
    Routing::unseen(client)
}

/// Runs `launch` with the recording proxy in front of the client's route, and says what the proxy saw. `launch` is
/// handed the environment its client is started in: the gateway's own, with the Pi home pointed at the per-run
/// copy. When the proxy cannot be set up (the home names no such provider, no port can be bound), `launch` runs
/// with the gateway's own environment and the routing is `hidden`: a failed setup never fails the run, since the
/// launch reports its own refusal.
pub fn recorded(runtime: &Runtime, curl: &str, dir: &str, provider: &str, client: &str, launch: &mut dyn FnMut(&HashMap<String, String>)) -> Routing {
    let proxy = TcpListener::bind("127.0.0.1:0").ok().and_then(|listener| {
        let port = listener.local_addr().map_or(0, |address| address.port());
        proxy_home(runtime, dir, provider, port).map(|origin| (listener, origin))
    });
    let Some((listener, origin)) = proxy else { return unproxied(runtime, dir, client, launch) };
    let _ = listener.set_nonblocking(true);
    let mut env = runtime.env.clone();
    env.insert("PI_CODING_AGENT_DIR".to_string(), dir.to_string());
    let seen: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
    let stop = AtomicBool::new(false);
    let upstream = Upstream { origin: &origin, dir, curl };
    std::thread::scope(|scope| {
        scope.spawn(|| serve(&listener, &stop, &upstream, &seen));
        launch(&env);
        stop.store(true, Ordering::Relaxed);
    });
    // The per-run home holds the route's key as the home does; it goes the moment the run ends.
    let _ = runtime.files.remove(dir);
    let noted = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    Routing::of(&noted)
}
