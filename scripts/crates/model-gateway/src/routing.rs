//! What a run's routing looked like to LiteLLM: which model served it and whether a fallback fired.
//!
//! LiteLLM says so in the headers of its responses (`x-litellm-model-id`, and `x-litellm-attempted-fallbacks`, an
//! integer count) and names a model in the body (which LiteLLM rewrites to the requested name unless a fallback
//! fired, so `served_model` is the requested name on a run that needed none). A delegate client talks to the proxy
//! itself and shows the gateway none of that, so for a tier run the gateway stands between them: it starts a small
//! recording proxy on `127.0.0.1`, points the client's route at it (a per-run copy of the Pi home whose provider has
//! the proxy as its base URL, so the confinement allows exactly that port), and forwards every request to the real
//! proxy with `curl`, which speaks TLS. Each response is passed back as it arrives, and its headers are noted.
//!
//! The proxy is not a door onto the owner's key. The per-run home holds a random token, not the key: the proxy
//! answers `401` to a request that does not carry that token, and it adds the real key itself. A request's own
//! headers are never passed on but `Content-Type` and `Accept`, whose values are quoted into a private `curl` config
//! file (never a command line); a request head with a control byte in it is refused.
//!
//! A client the gateway does not stand in front of records `hidden`, and one that never goes through LiteLLM
//! records `not-litellm`: the ledger says what it could not see, it does not guess.

use crate::json::{self, Object, Value};
use crate::pi_launch::{home_agent_dir, key_of};
use crate::ports::Runtime;
use crate::text::join;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// The clients that never call LiteLLM: their routing is not LiteLLM's to report.
const NOT_LITELLM: [&str; 3] = ["claude", "codex", "agy"];

/// The most of a request's head the proxy reads before it gives up on it.
const MAX_HEAD: usize = 64 * 1024;

/// How much of a response's body is searched for the model that answered.
const BODY_PREFIX: usize = 8 * 1024;

/// How long the proxy waits on a client that has gone quiet, before it drops the connection.
const IO_TIMEOUT: Duration = Duration::from_secs(60);

/// How the gateway knew a run's routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    /// The gateway saw LiteLLM's own response headers on at least one request.
    Recorded,
    /// The gateway stood in front of the client, and the client made no request that was answered.
    NoRequest,
    /// The gateway stood in front of the client and saw answers, none of which carried a LiteLLM header.
    NoHeaders,
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
            Capture::NoHeaders => "no-litellm-headers",
            Capture::Hidden => "hidden",
            Capture::NotLitellm => "not-litellm",
        }
    }
}

/// What the proxy noted of one answered request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seen {
    pub model_id: Option<String>,
    /// The upstream the proxy sent the request to: its `x-litellm-model-api-base` header.
    pub api_base: Option<String>,
    /// The largest integer any `x-litellm-attempted-fallbacks` line of the response carried.
    pub attempted_fallbacks: Option<u64>,
    /// The response had an `x-litellm-attempted-fallbacks` line, and none of them was an integer.
    pub fallbacks_unreadable: bool,
    pub served_model: Option<String>,
    /// The allowance of OpenCode Go's plan the request drew on: `usage.opencode.allowance` of the response body,
    /// in USD. `None` for a response that carries none (every route that is not OpenCode Go's).
    pub allowance_usd: Option<f64>,
    /// What LiteLLM priced the request at: its `x-litellm-response-cost` header, in USD.
    pub cost_usd: Option<f64>,
}

impl Seen {
    /// Whether the response carried a header of LiteLLM's that says anything of its routing.
    fn answered_by_litellm(&self) -> bool {
        self.model_id.is_some() || self.attempted_fallbacks.is_some() || self.fallbacks_unreadable
    }
}

/// A run's routing, as the ledger line writes it: `None` is a value the gateway could not see.
#[derive(Clone, Debug, PartialEq)]
pub struct Routing {
    pub capture: Capture,
    pub model_id: Option<String>,
    /// The upstream of the last request that named one.
    pub api_base: Option<String>,
    pub served_model: Option<String>,
    pub attempted_fallbacks: Option<u64>,
    /// The Go allowance the run's requests drew on, summed; `None` when no response carried one.
    pub allowance_usd: Option<f64>,
    /// What LiteLLM priced the run's requests at, summed; `None` when no response carried a price.
    pub litellm_cost_usd: Option<f64>,
}

impl Routing {
    /// The routing of a run the gateway did not stand in front of: nothing seen, and why.
    pub fn unseen(client: &str) -> Routing {
        let capture = if NOT_LITELLM.contains(&client) { Capture::NotLitellm } else { Capture::Hidden };
        Routing { capture, model_id: None, api_base: None, served_model: None, attempted_fallbacks: None, allowance_usd: None, litellm_cost_usd: None }
    }

    /// The routing of what the proxy saw: the last answered request names the serving model, and the most
    /// fallbacks any request needed is the run's. A response with a LiteLLM id and no fallbacks line needed none; a
    /// fallbacks line that is no integer leaves the count unknown, never zero; and answers with no LiteLLM header at
    /// all are `no-litellm-headers`, not a recording.
    pub fn of(seen: &[Seen]) -> Routing {
        let Some(last) = seen.last() else {
            return Routing { capture: Capture::NoRequest, model_id: None, api_base: None, served_model: None, attempted_fallbacks: None, allowance_usd: None, litellm_cost_usd: None };
        };
        let served_model = last.served_model.clone();
        if !seen.iter().any(Seen::answered_by_litellm) {
            return Routing { capture: Capture::NoHeaders, model_id: None, api_base: None, served_model, attempted_fallbacks: None, allowance_usd: sum_of(seen, |one| one.allowance_usd), litellm_cost_usd: sum_of(seen, |one| one.cost_usd) };
        }
        let counted = seen.iter().filter_map(|one| one.attempted_fallbacks).max();
        let unreadable = seen.iter().any(|one| one.fallbacks_unreadable);
        Routing {
            capture: Capture::Recorded,
            model_id: seen.iter().rev().find_map(|one| one.model_id.clone()),
            api_base: seen.iter().rev().find_map(|one| one.api_base.clone()),
            served_model,
            attempted_fallbacks: counted.or(if unreadable { None } else { Some(0) }),
            allowance_usd: sum_of(seen, |one| one.allowance_usd),
            litellm_cost_usd: sum_of(seen, |one| one.cost_usd),
        }
    }

    /// What the run's result and its `routing` event say: how the gateway knew the route, the serving model and
    /// deployment as far as LiteLLM's headers named them, and whether the proxy fell back to another one (null
    /// where the gateway could not see).
    pub fn report(&self, client: &str, model: &str, gateway_fallback: bool) -> Value {
        let seen = |value: Option<Value>| value.unwrap_or(Value::Null);
        json::object(vec![
            ("api_base", seen(self.api_base.as_deref().map(Value::str))),
            ("attempted_fallbacks", seen(self.attempted_fallbacks.map(|count| Value::Num(count as f64)))),
            ("capture", Value::str(self.capture.as_str())),
            ("fallback_fired", seen(self.fallback_fired().map(Value::Bool))),
            ("gateway_fallback", Value::Bool(gateway_fallback)),
            ("litellm_model_id", seen(self.model_id.as_deref().map(Value::str))),
            ("requested", Value::Str(format!("{client} {model}"))),
            ("served_model", seen(self.served_model.as_deref().map(Value::str))),
        ])
    }

    /// Whether the gateway saw the client's requests answered, and so has something to say of the route.
    pub fn watched(&self) -> bool {
        matches!(self.capture, Capture::Recorded | Capture::NoHeaders)
    }

    /// Whether LiteLLM fell back to another deployment, or `None` when the gateway could not tell.
    pub fn fallback_fired(&self) -> Option<bool> {
        self.attempted_fallbacks.map(|count| count >= 1)
    }
}

/// The sum of one reading over the requests that carried it; `None` when none did.
fn sum_of(seen: &[Seen], reading: impl Fn(&Seen) -> Option<f64>) -> Option<f64> {
    let readings: Vec<f64> = seen.iter().filter_map(reading).collect();
    (!readings.is_empty()).then(|| readings.iter().sum())
}

/// The allowance a response body reports: the number after the last `"allowance":` in `body`. OpenCode Go puts it in
/// `usage.opencode`, in the JSON body or in the last event of a stream, so the last one is the request's.
pub fn allowance_of(body: &str) -> Option<f64> {
    let key = "\"allowance\"";
    let at = body.rfind(key)? + key.len();
    let rest = body[at..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(rest.len());
    rest[..end].parse::<f64>().ok().filter(|number| number.is_finite() && *number >= 0.0)
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

/// Every value of a header, by name in any case, in the order they came.
fn header_values<'a>(headers: &'a [(String, String)], name: &'a str) -> impl Iterator<Item = &'a str> {
    headers.iter().filter(move |(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
}

/// What a response's headers and the start of its body say of the routing. A header that comes twice is read as
/// the first non-empty id and the largest integer count; a count that is no integer is not a count.
pub fn seen_of(headers: &[(String, String)], body_prefix: &str) -> Seen {
    let counts: Vec<&str> = header_values(headers, "x-litellm-attempted-fallbacks").collect();
    let parsed = counts.iter().filter_map(|count| count.parse::<u64>().ok()).max();
    Seen {
        model_id: header_values(headers, "x-litellm-model-id").find(|id| !id.is_empty()).map(str::to_string),
        api_base: header_values(headers, "x-litellm-model-api-base").find(|base| !base.is_empty()).map(str::to_string),
        attempted_fallbacks: parsed,
        fallbacks_unreadable: !counts.is_empty() && parsed.is_none(),
        served_model: served_model_of(body_prefix),
        allowance_usd: allowance_of(body_prefix),
        cost_usd: header_values(headers, "x-litellm-response-cost").find_map(|value| value.parse::<f64>().ok()).filter(|cost| cost.is_finite() && *cost >= 0.0),
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

/// Whether a head holds a byte that has no place in one: a control byte other than the CR LF that end each line and
/// a tab, or a CR or LF that is not part of a CR LF.
fn has_control_byte(head: &[u8]) -> bool {
    head.iter().enumerate().any(|(at, byte)| {
        let line_end = (*byte == b'\r' && head.get(at + 1) == Some(&b'\n')) || (*byte == b'\n' && at > 0 && head[at - 1] == b'\r');
        (*byte < 0x20 && *byte != b'\t' && !line_end) || *byte == 0x7f
    })
}

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
    if has_control_byte(&buffer[..end]) {
        return Err(Refusal(400, "the request head holds a control byte".into()));
    }
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    let mut first = head.split("\r\n").next().unwrap_or("").split(' ');
    let (Some(method), Some(target)) = (first.next(), first.next()) else {
        return Err(Refusal(400, "the request line is not a method and a target".into()));
    };
    if method.is_empty() || !method.bytes().all(|byte| byte.is_ascii_uppercase()) || !target.starts_with('/') {
        return Err(Refusal(400, "the request line is not a method and an absolute path".into()));
    }
    let headers = header_lines(&head);
    if header_values(&headers, "transfer-encoding").next().is_some() {
        return Err(Refusal(411, "a chunked request body is not supported: send a content-length".into()));
    }
    let length = header_values(&headers, "content-length").next().and_then(|value| value.parse::<usize>().ok()).unwrap_or(0);
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

/// The request headers that are passed on to LiteLLM: nothing else of the client's reaches it.
const FORWARDED: [&str; 2] = ["content-type", "accept"];

/// The headers a relayed response does not carry: the proxy ends the body by closing the connection.
const NOT_RELAYED: [&str; 4] = ["content-length", "connection", "transfer-encoding", "keep-alive"];

static CONFIGS: AtomicUsize = AtomicUsize::new(0);

/// What the proxy forwards with: where the real proxy is, where its private files go, what program speaks to it,
/// the real key it adds, and the token a client must present.
struct Upstream {
    origin: String,
    dir: String,
    curl: String,
    key: String,
    token: String,
    timeout: Duration,
}

/// A value as a curl config file reads it between double quotes: backslash and quote escaped.
pub(crate) fn quoted(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A file that is removed when it goes out of scope, whichever way the request ends.
struct Scratch(String);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A curl config holding the headers of a forward (the real key, and the client's content type and accept), so the
/// key is in a file only its owner reads and not on a command line; `None` when the file cannot be made.
fn curl_config(upstream: &Upstream, request: &Request) -> Option<Scratch> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = format!("{}/curl-{}.conf", upstream.dir, CONFIGS.fetch_add(1, Ordering::Relaxed));
    let mut text = format!("header = \"Authorization: Bearer {}\"\n", quoted(&upstream.key));
    for (key, value) in request.headers.iter().filter(|(key, _)| FORWARDED.contains(&key.to_ascii_lowercase().as_str())) {
        text.push_str(&format!("header = \"{}: {}\"\n", quoted(&key.to_ascii_lowercase()), quoted(value)));
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).ok()?;
    let scratch = Scratch(path);
    file.write_all(text.as_bytes()).ok()?;
    Some(scratch)
}

/// Whether two strings are equal, in time that does not say where they first differ.
fn same(left: &str, right: &str) -> bool {
    left.len() == right.len() && left.bytes().zip(right.bytes()).fold(0u8, |all, (a, b)| all | (a ^ b)) == 0
}

/// Whether the request carries the run's token as its bearer.
fn authorised(upstream: &Upstream, request: &Request) -> bool {
    let wanted = format!("Bearer {}", upstream.token);
    header_values(&request.headers, "authorization").any(|given| same(given, &wanted))
}

/// Forwards one request and relays the answer, noting the routing headers on the way. A request that cannot be
/// forwarded is answered with the status that says so.
fn forward(stream: &mut TcpStream, request: &Request, upstream: &Upstream, seen: &Mutex<Vec<Seen>>, stop: &AtomicBool) -> Result<(), Refusal> {
    let config = curl_config(upstream, request).ok_or_else(|| Refusal(502, "the proxy cannot write its curl config".to_string()))?;
    let url = format!("{}{}", upstream.origin, request.target);
    let spawned = Command::new(&upstream.curl)
        .args(["-sS", "-i", "--no-buffer", "--http1.1", "--connect-timeout", "30", "--speed-limit", "1", "--speed-time", "300", "--config", &config.0, "-X", &request.method, "--data-binary", "@-", "--", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = spawned.map_err(|error| Refusal(502, format!("the proxy cannot start {}: {error}", upstream.curl)))?;
    // The body goes in from a thread of its own: a body larger than the pipes would otherwise stop curl, which is
    // not yet reading its answer, and the proxy with it.
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let body = request.body.clone();
    std::thread::spawn(move || stdin.write_all(&body));
    let mut out = child.stdout.take().expect("stdout was piped");
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    // The answer's head: a `100 Continue` before it is skipped.
    let (status_line, headers, rest) = loop {
        let Some(end) = head_end(&buffer) else {
            match out.read(&mut chunk) {
                Ok(0) | Err(_) => {
                    let said = child.wait_with_output().map(|done| String::from_utf8_lossy(&done.stderr).trim().to_string()).unwrap_or_default();
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
    drop(config);
    let code = status_line.split(' ').nth(1).unwrap_or("502");
    let reason = status_line.splitn(3, ' ').nth(2).unwrap_or("");
    // The headers are noted before the client is told anything of the answer, so a client that has read the answer
    // and goes away is already in the record; the model the body names is added once more of it has been read.
    let named = |prefix: &[u8], tail: &[u8]| {
        let mut one = seen_of(&headers, &String::from_utf8_lossy(&prefix[..prefix.len().min(BODY_PREFIX)]));
        // A stream reports its usage in its last event, which the prefix has not reached.
        one.allowance_usd = one.allowance_usd.or_else(|| allowance_of(&String::from_utf8_lossy(tail)));
        one
    };
    let at = {
        let mut noted = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        noted.push(named(&rest, &rest));
        noted.len() - 1
    };
    let mut head = format!("HTTP/1.1 {code} {reason}\r\n");
    for (key, value) in headers.iter().filter(|(key, _)| !NOT_RELAYED.contains(&key.to_ascii_lowercase().as_str())) {
        head.push_str(&format!("{key}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    let mut relayed = stream.write_all(head.as_bytes()).is_ok();
    let mut prefix: Vec<u8> = rest.clone();
    let mut tail: Vec<u8> = rest.clone();
    relayed = relayed && stream.write_all(&rest).is_ok();
    while relayed && !stop.load(Ordering::Relaxed) {
        match out.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if prefix.len() < BODY_PREFIX {
                    prefix.extend_from_slice(&chunk[..count]);
                }
                tail.extend_from_slice(&chunk[..count]);
                if tail.len() > 2 * BODY_PREFIX {
                    tail.drain(..tail.len() - BODY_PREFIX);
                }
                relayed = stream.write_all(&chunk[..count]).is_ok();
            }
        }
    }
    if !relayed || stop.load(Ordering::Relaxed) {
        let _ = child.kill();
    }
    let _ = child.wait();
    seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner)[at] = named(&prefix, &tail);
    Ok(())
}

/// Answers one connection: one request, checked, forwarded, then the connection closes. A client that goes quiet is
/// dropped after the timeout.
fn serve_one(mut stream: TcpStream, upstream: &Upstream, seen: &Mutex<Vec<Seen>>, stop: &AtomicBool) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(upstream.timeout));
    let _ = stream.set_write_timeout(Some(upstream.timeout));
    let outcome = read_request(&mut stream).and_then(|request| {
        if authorised(upstream, &request) { forward(&mut stream, &request, upstream, seen, stop) } else { Err(Refusal(401, "the request does not carry this run's token".into())) }
    });
    if let Err(Refusal(status, message)) = outcome {
        let body = json::compact(&json::object(vec![("error", Value::str(&message))]));
        let _ = write!(stream, "HTTP/1.1 {status} Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    }
}

/// Where a provider's real proxy is, from the home's `models.json`: the origin (`https://host`), and the path
/// its base URL carries (`/v1`).
pub fn split_base(base_url: &str) -> Option<(String, String)> {
    let scheme = base_url.find("://")? + 3;
    let slash = base_url[scheme..].find('/').map_or(base_url.len(), |at| scheme + at);
    Some((base_url[..slash].to_string(), base_url[slash..].to_string()))
}

/// A random token: sixteen bytes from the system's source, in hex; `None` when it cannot be read.
fn random_token() -> Option<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom").ok()?.read_exact(&mut bytes).ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// What the per-run home was made from: the real proxy's origin, and the key the proxy adds.
struct Home {
    origin: String,
    key: String,
}

/// The per-run Pi home that points the provider at the proxy: its `models.json` is the provider's entry with the
/// proxy as its base URL and the run's token as its key, and the home's `bin` is copied beside it. The real key is
/// read here, by the gateway, and never written into the run's home. `None` when the home names no such provider,
/// the key cannot be read or holds a control byte, or the per-run home cannot be written.
fn proxy_home(runtime: &Runtime, dir: &str, provider: &str, port: u16, token: &str) -> Option<Home> {
    let home = home_agent_dir(&runtime.env);
    let parsed = json::parse(&runtime.files.read_text(&join(&[&home, "models.json"])).ok()?).ok()?;
    let entry = parsed.as_object()?.get("providers")?.as_object()?.get(provider)?.as_object()?;
    let (origin, path) = split_base(entry.get("baseUrl")?.as_str()?)?;
    let key = key_of(runtime.process.as_ref(), provider, entry.get("apiKey")?.as_str()?).ok()?;
    if key.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return None;
    }
    let mut recorded: Object = entry.clone();
    recorded.set("baseUrl", Value::Str(format!("http://127.0.0.1:{port}{path}")));
    recorded.set("apiKey", Value::str(token));
    let models = json::object(vec![("providers", json::object(vec![(provider, Value::Obj(recorded))]))]);
    runtime.files.write_private(&join(&[dir, "models.json"]), &format!("{}\n", json::pretty(&models))).ok()?;
    let bin = join(&[&home, "bin"]);
    if runtime.files.exists(&bin).unwrap_or(false) {
        runtime.files.copy_tree(&bin, &join(&[dir, "bin"])).ok()?;
    }
    Some(Home { origin, key })
}

/// The launch with the gateway's own environment, when no proxy stands in front of it: its routing is unseen.
fn unproxied(runtime: &Runtime, dir: &str, client: &str, launch: &mut dyn FnMut(&HashMap<String, String>)) -> Routing {
    let _ = runtime.files.remove(dir);
    launch(&runtime.env);
    Routing::unseen(client)
}

/// A running proxy. However the run ends, including a launch that panics, dropping it stops the proxy, waits for
/// its accept thread (connections already taken are bound by their timeouts, not waited for) and removes the
/// per-run home, which holds the token and the curl configs.
struct Proxy {
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    dir: String,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.accept.take().map(JoinHandle::join);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `recorded` with the proxy's idle timeout chosen by the caller.
pub fn recorded_within(runtime: &Runtime, curl: &str, dir: &str, provider: &str, client: &str, timeout: Duration, launch: &mut dyn FnMut(&HashMap<String, String>)) -> Routing {
    let token = random_token();
    let proxy = TcpListener::bind("127.0.0.1:0").ok().zip(token).and_then(|(listener, token)| {
        let port = listener.local_addr().map_or(0, |address| address.port());
        proxy_home(runtime, dir, provider, port, &token).map(|home| (listener, home, token))
    });
    let Some((listener, home, token)) = proxy else { return unproxied(runtime, dir, client, launch) };
    let _ = listener.set_nonblocking(true);
    let mut env = runtime.env.clone();
    env.insert("PI_CODING_AGENT_DIR".to_string(), dir.to_string());
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let upstream = Arc::new(Upstream { origin: home.origin, dir: dir.to_string(), curl: curl.to_string(), key: home.key, token, timeout });
    let accept = {
        let (stop, seen, upstream) = (stop.clone(), seen.clone(), upstream.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (stop, seen, upstream) = (stop.clone(), seen.clone(), upstream.clone());
                        std::thread::spawn(move || serve_one(stream, &upstream, &seen, &stop));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        })
    };
    let running = Proxy { stop, accept: Some(accept), dir: dir.to_string() };
    launch(&env);
    drop(running);
    let noted = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    Routing::of(&noted)
}

/// Runs `launch` with the recording proxy in front of the client's route, and says what the proxy saw. `launch` is
/// handed the environment its client is started in: the gateway's own, with the Pi home pointed at the per-run
/// copy. When the proxy cannot be set up (the home names no such provider, the key cannot be read, no port can be
/// bound), `launch` runs with the gateway's own environment and the routing is `hidden`: a failed setup never fails
/// the run, since the launch reports its own refusal.
pub fn recorded(runtime: &Runtime, curl: &str, dir: &str, provider: &str, client: &str, launch: &mut dyn FnMut(&HashMap<String, String>)) -> Routing {
    recorded_within(runtime, curl, dir, provider, client, IO_TIMEOUT, launch)
}
