//! The node-side LiteLLM budget gate.
//!
//! A Gold Band node runs `litellm-gate check` before work. It makes ONE tiny
//! chat-completion probe with the node's LiteLLM virtual key and fails with
//! exit code 1 when the proxy refuses it because the key's budget is
//! exhausted, or when the proxy cannot be asked at all. The key is read from the
//! environment and handed to `curl` through a config on its stdin, so no
//! secret is ever in an argv, and neither the key, the edge token nor the
//! response body is ever printed.
//!
//! std only, like agent-kit-setup: the probe spawns `curl`.

use std::ffi::OsString;
use std::io::Write;
use std::process::{Command, Stdio};

pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The header the key travels in unless `--header` says otherwise.
const DEFAULT_HEADER: &str = "x-litellm-api-key";

/// The fixed curl arguments; everything else, secrets included, goes on stdin.
const CURL_ARGS: [&str; 5] = ["--silent", "--show-error", "--max-time", "30", "--config"];

/// What the proxy said about the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The probe was answered: the key is within its budget.
    Within,
    /// LiteLLM refused the probe because the key's budget is spent.
    BudgetExhausted,
    /// The proxy refused the probe for another reason (the HTTP status).
    Refused(u16),
    /// No usable answer: curl failed, was missing, or the status was unreadable.
    Unreachable,
}

/// Classifies one proxy answer. LiteLLM's real refusal is HTTP 400 with
/// `{"error":{"message":"Budget has been exceeded! ...","type":"budget_exceeded"}}`;
/// 429 with the same body is accepted too.
pub fn classify(status: u16, body: &str) -> Verdict {
    let spent = [
        "budget_exceeded",
        "ExceededBudget",
        "Budget has been exceeded",
    ]
    .iter()
    .any(|marker| body.contains(marker));
    match status {
        200..=299 => Verdict::Within,
        400 | 429 if spent => Verdict::BudgetExhausted,
        other => Verdict::Refused(other),
    }
}

/// Everything `check` needs, parsed from the command line.
#[derive(Debug, PartialEq, Eq)]
struct Options {
    node: String,
    url: String,
    key_env: String,
    model: String,
    header: String,
    edge_token_env: Option<String>,
}

/// The usage text a bad command line prints.
fn usage() -> String {
    format!(
        "usage: {NAME} --version\n       {NAME} check --node <name> --url <base-url> \
         --key-env <ENV> --model <model> [--header <name>] [--edge-token-env <ENV>]"
    )
}

fn parse(args: Vec<OsString>) -> Result<Options, String> {
    let mut args = args.into_iter().map(OsString::into_string);
    let first = args.next().transpose().map_err(|_| "non-UTF-8 argument")?;
    if first.as_deref() != Some("check") {
        return Err("expected the command `check`".into());
    }
    let (mut node, mut url, mut key_env, mut model) = (None, None, None, None);
    let (mut header, mut edge) = (None, None);
    while let Some(flag) = args.next() {
        let flag = flag.map_err(|_| "non-UTF-8 argument")?;
        let slot = match flag.as_str() {
            "--node" => &mut node,
            "--url" => &mut url,
            "--key-env" => &mut key_env,
            "--model" => &mut model,
            "--header" => &mut header,
            "--edge-token-env" => &mut edge,
            _ => return Err("unknown argument".into()),
        };
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        *slot = Some(value.map_err(|_| "non-UTF-8 argument")?);
    }
    let header = header.unwrap_or_else(|| DEFAULT_HEADER.to_string());
    if header.is_empty()
        || !header
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("--header must be letters, digits and `-`".into());
    }
    let need = |value: Option<String>, flag: &str| value.ok_or(format!("{flag} is required"));
    Ok(Options {
        node: need(node, "--node")?,
        url: need(url, "--url")?,
        key_env: need(key_env, "--key-env")?,
        model: need(model, "--model")?,
        header,
        edge_token_env: edge,
    })
}

/// True when every character of `value` is in `allowed` (and it is not empty).
fn only(value: &str, allowed: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || allowed.contains(c))
}

/// A secret is a LiteLLM key or bearer token: `[A-Za-z0-9._~+/=-]`.
fn secret_is_safe(secret: &str) -> bool {
    only(secret, "._~+/=-")
}

/// A model name: `[A-Za-z0-9._:/-]`.
fn model_is_safe(model: &str) -> bool {
    only(model, "._:/-")
}

/// A base URL must be https, or http to the loopback host only (stubs), and
/// free of anything a curl config could misread.
fn url_is_safe(url: &str) -> bool {
    let scheme_ok = url.starts_with("https://")
        || url.strip_prefix("http://").is_some_and(|rest| {
            let host = rest.split(['/', ':']).next();
            matches!(host, Some("127.0.0.1" | "localhost"))
        });
    scheme_ok
        && url
            .chars()
            .all(|c| c.is_ascii_graphic() && c != '"' && c != '\\')
}

/// An environment variable name: `[A-Z][A-Z0-9_]*`. Anything else is never
/// echoed back; messages say `<invalid name>` instead.
fn name_is_valid(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// The name as it may be shown.
fn shown(name: &str) -> &str {
    if name_is_valid(name) {
        name
    } else {
        "<invalid name>"
    }
}

/// The JSON probe body; the model name has been vetted so it needs no escaping
/// beyond what `model_is_safe` already excludes.
fn probe_body(model: &str) -> String {
    format!(
        "{{\"model\":\"{model}\",\"max_tokens\":1,\"messages\":[{{\"role\":\"user\",\"content\":\"ping\"}}]}}"
    )
}

/// Escapes a value for a double-quoted curl config string.
fn config_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Builds the curl config written to curl's stdin. It carries the URL, the
/// key header and the optional edge bearer, so none of them is in an argv.
pub fn curl_config(
    base_url: &str,
    header: &str,
    key: &str,
    edge_token: Option<&str>,
    model: &str,
) -> String {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let mut config = format!("url = {}\nrequest = \"POST\"\n", config_quote(&url));
    config += &format!("header = {}\n", config_quote(&format!("{header}: {key}")));
    if let Some(token) = edge_token {
        let bearer = format!("Authorization: Bearer {token}");
        config += &format!("header = {}\n", config_quote(&bearer));
    }
    config += "header = \"Content-Type: application/json\"\n";
    config += &format!("data = {}\n", config_quote(&probe_body(model)));
    config += "write-out = \"\\n%{http_code}\"\n";
    config
}

/// Runs `program` with curl's arguments and `config` on stdin; returns its
/// stdout, or None when it could not run or exited non-zero.
fn fetch_with(program: &str, config: &str) -> Option<String> {
    let mut child = Command::new(program)
        .args(CURL_ARGS)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        // A curl that dies early closes the pipe; its exit status says so.
        let _ = stdin.write_all(config.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Splits curl's output (`<body>\n<status>`) and classifies it.
fn verdict_of(output: Option<String>) -> Verdict {
    let parsed = output.as_deref().and_then(|text| {
        let (body, status) = text.rsplit_once('\n')?;
        Some((status.trim().parse::<u16>().ok()?, body))
    });
    match parsed {
        Some((status, body)) => classify(status, body),
        None => Verdict::Unreachable,
    }
}

/// Reads a secret named by `var`; the error line names the variable only.
fn read_secret<E>(env: &E, var: &str) -> Result<String, String>
where
    E: Fn(&str) -> Option<String>,
{
    if !name_is_valid(var) {
        return Err("environment variable name is not [A-Z][A-Z0-9_]*".into());
    }
    let value = env(var).filter(|v| !v.is_empty());
    let value = value.ok_or(format!("environment variable {var} is not set or is empty"))?;
    if secret_is_safe(&value) {
        Ok(value)
    } else {
        Err(format!(
            "environment variable {var} holds characters outside [A-Za-z0-9._~+/=-]"
        ))
    }
}

/// Runs the probe described by `options` and returns the verdict, or the
/// reason it could not be made (exit 1, naming no secret).
fn check<E>(options: &Options, env: &E, program: &str) -> Result<Verdict, String>
where
    E: Fn(&str) -> Option<String>,
{
    if !model_is_safe(&options.model) {
        return Err("--model must be letters, digits and . _ : / -".into());
    }
    if !url_is_safe(&options.url) {
        return Err(
            "--url must be https (or http to 127.0.0.1/localhost) without quotes or spaces".into(),
        );
    }
    let key = read_secret(env, &options.key_env)?;
    let edge = match &options.edge_token_env {
        Some(var) => Some(read_secret(env, var)?),
        None => None,
    };
    let config = curl_config(
        &options.url,
        &options.header,
        &key,
        edge.as_deref(),
        &options.model,
    );
    Ok(verdict_of(fetch_with(program, &config)))
}

/// The line and exit code a result earns.
fn report(options: &Options, result: Result<Verdict, String>) -> (String, i32) {
    let node = &options.node;
    let var = shown(&options.key_env);
    let failed = |why: String| (format!("{NAME}: node {node} failed: {why}"), 1);
    match result {
        Ok(Verdict::Within) => (
            format!("{NAME}: node {node}: key {var} is within its budget"),
            0,
        ),
        Ok(Verdict::BudgetExhausted) => failed(format!("budget exhausted for key {var}")),
        Ok(Verdict::Refused(status)) => failed(format!(
            "proxy refused the request (HTTP {status}) for key {var}"
        )),
        Ok(Verdict::Unreachable) => failed(format!("proxy unreachable for key {var}")),
        Err(reason) => failed(format!("{reason} (key {var})")),
    }
}

/// Runs one command line with `env` as the environment lookup and returns the
/// exit code: 0 within budget (or `--version`), 1 failed, 2 bad usage.
pub fn run_with<I, E>(args: I, env: E) -> i32
where
    I: IntoIterator<Item = OsString>,
    E: Fn(&str) -> Option<String>,
{
    run_program(args, env, "curl")
}

/// Answers one command line: the line to print and the exit code it earns.
fn execute<I, E>(args: I, env: E, program: &str) -> (String, i32)
where
    I: IntoIterator<Item = OsString>,
    E: Fn(&str) -> Option<String>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    if args.first().is_some_and(|a| a == "--version") {
        return (format!("{NAME} {}", env!("CARGO_PKG_VERSION")), 0);
    }
    match parse(args) {
        Ok(options) => report(&options, check(&options, &env, program)),
        Err(why) => (format!("{NAME}: {why}\n{}", usage()), 2),
    }
}

fn run_program<I, E>(args: I, env: E, program: &str) -> i32
where
    I: IntoIterator<Item = OsString>,
    E: Fn(&str) -> Option<String>,
{
    let (message, code) = execute(args, env, program);
    if code == 0 {
        println!("{message}");
    } else {
        eprintln!("{message}");
    }
    code
}

/// Runs one command line against the process environment.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    run_with(args, |var| std::env::var(var).ok())
}

// The unit tests below execute every line of this file, which is the
// workspace's 100 percent line-coverage standard (docs/RUST.md); the
// executable line in main.rs, the exit wrapper, is the one line they cannot.
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    const BUDGET_BODY: &str = r#"{"error":{"message":"Budget has been exceeded! Current cost: 5.1, Max budget: 5.0","type":"budget_exceeded","param":null,"code":"400"}}"#;
    const KEY: &str = "sk-test_KEY.123";

    /// A one-shot stub proxy: replies with `status`/`body` and yields the raw request.
    fn stub(status: u16, body: &'static str) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            // Bounded: a client that never connects must not hang the test.
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => return String::new(),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&request).into_owned();
                if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .to_ascii_lowercase()
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: ").map(str::to_string))
                        .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                    if rest.len() >= length {
                        break;
                    }
                }
            }
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
            String::from_utf8_lossy(&request).into_owned()
        });
        (url, handle)
    }

    fn env(var: &str) -> Option<String> {
        match var {
            "LITELLM_KEY" => Some(KEY.to_string()),
            "EDGE_TOKEN" => Some("edge.tok-1".to_string()),
            "EMPTY" => Some(String::new()),
            "BAD" => Some("sk bad\"key".to_string()),
            _ => None,
        }
    }

    fn args(extra: &[&str]) -> Vec<OsString> {
        let mut all: Vec<OsString> = ["check", "--node", "n1", "--model", "gpt-4o"]
            .iter()
            .map(OsString::from)
            .collect();
        all.extend(extra.iter().map(OsString::from));
        all
    }

    fn check_args(url: &str, extra: &[&str]) -> Vec<OsString> {
        let mut all = args(&["--url", url, "--key-env", "LITELLM_KEY"]);
        all.extend(extra.iter().map(OsString::from));
        all
    }

    #[test]
    fn below_cap_exits_zero_and_sends_the_key_only_in_the_header() {
        let (url, server) = stub(200, r#"{"choices":[]}"#);
        assert_eq!(run_with(check_args(&url, &[]), env), 0);
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request.contains(&format!("x-litellm-api-key: {KEY}")));
        assert!(!request.contains("Authorization"));
        assert!(request.contains(r#""model":"gpt-4o""#));
        assert!(request.contains(r#""max_tokens":1"#));
    }

    /// The key variable the registry's route table names for an agent id.
    fn route_key_env(agent: &str) -> String {
        let table = include_str!("../../../../registry/generated/litellm/routes.json");
        let from = table
            .find(&format!("\"agent\": \"{agent}\""))
            .unwrap_or_else(|| panic!("no route for {agent}"));
        let rest = &table[from..];
        let key = rest.find("\"key_env\": \"").unwrap() + "\"key_env\": \"".len();
        rest[key..key + rest[key..].find('"').unwrap()].to_string()
    }

    #[test]
    fn the_openrouter_route_is_gated_and_the_free_local_route_passes() {
        let openrouter = route_key_env("openrouter");
        let local = route_key_env("local");
        assert_eq!(openrouter, "LITELLM_OPENROUTER_KEY");
        assert_eq!(local, "LITELLM_LOCAL_KEY");
        let lookup = |var: &str| (var == openrouter || var == local).then(|| KEY.to_string());
        // The 5 EUR key, spent, fails the node and names its variable.
        let (url, server) = stub(400, BUDGET_BODY);
        let (message, code) = execute(
            args(&[
                "--url",
                &url,
                "--key-env",
                &openrouter,
                "--model",
                "openrouter/vendor/m:free",
            ]),
            lookup,
            "curl",
        );
        assert_eq!(code, 1);
        assert_eq!(
            message,
            format!("litellm-gate: node n1 failed: budget exhausted for key {openrouter}")
        );
        let request = server.join().unwrap();
        assert!(request.contains(r#""model":"openrouter/vendor/m:free""#));
        // The local route has no budget, so the proxy answers and the gate passes.
        let (url, server) = stub(200, r#"{"choices":[]}"#);
        assert_eq!(
            run_with(
                args(&[
                    "--url",
                    &url,
                    "--key-env",
                    &local,
                    "--model",
                    "local/qwen3:32b"
                ]),
                lookup
            ),
            0
        );
        server.join().unwrap();
    }

    #[test]
    fn exhausted_budget_fails_and_the_same_stub_within_budget_passes() {
        let (url, server) = stub(400, BUDGET_BODY);
        let (message, code) = execute(check_args(&url, &[]), env, "curl");
        assert_eq!(code, 1);
        assert_eq!(
            message,
            "litellm-gate: node n1 failed: budget exhausted for key LITELLM_KEY"
        );
        server.join().unwrap();
        let (url, server) = stub(400, r#"{"error":{"message":"bad request"}}"#);
        let (message, code) = execute(check_args(&url, &[]), env, "curl");
        assert_eq!(code, 1);
        assert!(message.contains("refused") && message.contains("HTTP 400"));
        assert!(!message.contains("budget exhausted"));
        server.join().unwrap();
        let (url, server) = stub(200, "{}");
        assert_eq!(run_with(check_args(&url, &[]), env), 0);
        server.join().unwrap();
    }

    #[test]
    fn a_429_budget_body_fails() {
        let (url, server) = stub(429, BUDGET_BODY);
        assert_eq!(run_with(check_args(&url, &[]), env), 1);
        server.join().unwrap();
    }

    #[test]
    fn a_server_error_is_refused() {
        let (url, server) = stub(500, "boom");
        let (message, code) = execute(check_args(&url, &[]), env, "curl");
        assert_eq!(code, 1);
        assert!(message.contains("refused"));
        server.join().unwrap();
        assert_eq!(
            report(
                &parse(check_args(&url, &[])).unwrap(),
                Ok(Verdict::Refused(500))
            )
            .0,
            "litellm-gate: node n1 failed: proxy refused the request (HTTP 500) for key LITELLM_KEY"
        );
    }

    #[test]
    fn a_refused_connection_is_unreachable() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}/v1");
        assert_eq!(run_with(check_args(&url, &[]), env), 1);
        let options = parse(check_args(&url, &[])).unwrap();
        assert_eq!(check(&options, &env, "curl").unwrap(), Verdict::Unreachable);
    }

    #[test]
    fn a_missing_curl_is_unreachable() {
        let options = parse(check_args("http://127.0.0.1:1/v1", &[])).unwrap();
        let result = check(&options, &env, "no-such-curl-litellm-gate");
        assert_eq!(result, Ok(Verdict::Unreachable));
        assert_eq!(
            report(&options, result).0,
            "litellm-gate: node n1 failed: proxy unreachable for key LITELLM_KEY"
        );
        assert_eq!(
            run_program(
                check_args("http://127.0.0.1:1/v1", &[]),
                env,
                "no-such-curl-litellm-gate"
            ),
            1
        );
    }

    #[test]
    fn a_curl_that_fails_is_unreachable() {
        assert_eq!(fetch_with("false", ""), None);
    }

    #[test]
    fn missing_or_empty_env_names_the_variable() {
        let mut a = args(&["--url", "http://127.0.0.1:1/v1", "--key-env", "NOPE"]);
        assert_eq!(run_with(a.clone(), env), 1);
        let options = parse(a.clone()).unwrap();
        let why = check(&options, &env, "curl").unwrap_err();
        assert!(why.contains("NOPE"));
        a = args(&["--url", "http://127.0.0.1:1/v1", "--key-env", "EMPTY"]);
        assert_eq!(run_with(a, env), 1);
        let a = check_args("http://127.0.0.1:1/v1", &["--edge-token-env", "NOPE2"]);
        assert_eq!(run_with(a, env), 1);
    }

    #[test]
    fn unsafe_secret_characters_fail_without_printing_the_value() {
        let a = args(&["--url", "http://127.0.0.1:1/v1", "--key-env", "BAD"]);
        assert_eq!(run_with(a.clone(), env), 1);
        let why = check(&parse(a).unwrap(), &env, "curl").unwrap_err();
        assert!(why.contains("BAD") && !why.contains("sk bad"));
    }

    #[test]
    fn unsafe_model_or_url_fails() {
        let a = vec![
            "check",
            "--node",
            "n1",
            "--model",
            "a\"b",
            "--url",
            "http://h/v1",
            "--key-env",
            "LITELLM_KEY",
        ];
        assert_eq!(run_with(a.iter().map(OsString::from), env), 1);
        let a = check_args("ftp://h/v1", &[]);
        assert_eq!(run_with(a, env), 1);
        let a = check_args("http://example.com/v1", &[]);
        assert_eq!(run_with(a, env), 1);
        assert!(url_is_safe("http://localhost:4000/v1") && url_is_safe("http://127.0.0.1/v1"));
        assert!(!url_is_safe("http://127.0.0.1.evil.com/v1"));
    }

    #[test]
    fn edge_token_and_custom_header_are_sent() {
        let (url, server) = stub(200, "{}");
        let a = check_args(
            &url,
            &["--header", "X-Custom-Key", "--edge-token-env", "EDGE_TOKEN"],
        );
        assert_eq!(run_with(a, env), 0);
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.contains(&format!("x-custom-key: {}", KEY.to_ascii_lowercase())));
        assert!(request.contains("authorization: bearer edge.tok-1"));
        assert!(!request.contains("x-litellm-api-key"));
    }

    #[test]
    fn bad_usage_exits_two() {
        let bad: Vec<Vec<OsString>> = vec![
            vec![],
            vec!["dance".into()],
            vec!["check".into()],
            args(&["--url"]),
            args(&["--bogus", "x"]),
            args(&["--url", "u", "--key-env", "K", "--header", "bad header"]),
            vec![OsString::from("check"), non_utf8()],
            vec![OsString::from("check"), "--node".into(), non_utf8()],
            vec![non_utf8()],
        ];
        for a in bad {
            assert_eq!(run_with(a.clone(), env), 2, "{a:?}");
        }
        assert!(usage().contains("check --node"));
    }

    #[test]
    fn unknown_flags_and_invalid_names_are_never_echoed() {
        let (message, code) = execute(args(&["--sk-secret-flag", "x"]), env, "curl");
        assert_eq!(code, 2);
        assert!(message.contains("unknown argument") && !message.contains("sk-secret"));
        let a = args(&["--url", "https://h/v1", "--key-env", "sk-secret value"]);
        let (message, code) = execute(a, env, "curl");
        assert_eq!(code, 1);
        assert!(message.contains("<invalid name>") && !message.contains("sk-secret"));
        let a = check_args("https://h/v1", &["--edge-token-env", "bad-name"]);
        let (message, code) = execute(a, env, "curl");
        assert_eq!(code, 1);
        assert!(!message.contains("bad-name"));
        assert!(name_is_valid("A1_B") && !name_is_valid("1A") && !name_is_valid("a"));
    }

    #[test]
    fn secrets_reach_curl_on_stdin_and_never_in_argv() {
        let dir = std::env::temp_dir().join(format!("litellm-gate-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (argv, stdin, fake) = (dir.join("argv"), dir.join("stdin"), dir.join("curl"));
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\nprintf '{{}}\\n200'\n",
            argv.display(),
            stdin.display()
        );
        std::fs::write(&fake, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let a = check_args("https://l.example/v1", &["--edge-token-env", "EDGE_TOKEN"]);
        let (message, code) = execute(a, env, fake.to_str().unwrap());
        assert_eq!((code, message.contains("within")), (0, true));
        let argv = std::fs::read_to_string(&argv).unwrap();
        assert!(argv.contains("--config") && argv.contains("--max-time"));
        assert!(!argv.contains(KEY) && !argv.contains("edge.tok-1"));
        let stdin = std::fs::read_to_string(&stdin).unwrap();
        assert!(stdin.contains(KEY) && stdin.contains("edge.tok-1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    fn non_utf8() -> OsString {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![0xff, 0xfe])
    }

    #[cfg(not(unix))]
    fn non_utf8() -> OsString {
        OsString::from("\u{fffd}")
    }

    #[test]
    fn version_exits_zero() {
        assert_eq!(run_with(vec!["--version".into()], env), 0);
        assert_eq!(run(vec!["--version".into()]), 0);
    }

    #[test]
    fn run_reads_the_process_environment() {
        let a = args(&[
            "--url",
            "http://127.0.0.1:1/v1",
            "--key-env",
            "LITELLM_GATE_UNSET_FOR_TESTS",
        ]);
        assert_eq!(run(a), 1);
    }

    #[test]
    fn classify_covers_every_verdict() {
        assert_eq!(classify(200, ""), Verdict::Within);
        assert_eq!(classify(299, "x"), Verdict::Within);
        assert_eq!(classify(400, BUDGET_BODY), Verdict::BudgetExhausted);
        assert_eq!(classify(429, "ExceededBudget"), Verdict::BudgetExhausted);
        assert_eq!(
            classify(400, "Budget has been exceeded"),
            Verdict::BudgetExhausted
        );
        assert_eq!(classify(400, "bad request"), Verdict::Refused(400));
        assert_eq!(classify(500, BUDGET_BODY), Verdict::Refused(500));
        assert_eq!(classify(401, ""), Verdict::Refused(401));
    }

    #[test]
    fn verdict_of_parses_the_last_line() {
        assert_eq!(verdict_of(Some("{}\n200".into())), Verdict::Within);
        assert_eq!(
            verdict_of(Some(format!("{BUDGET_BODY}\n400"))),
            Verdict::BudgetExhausted
        );
        assert_eq!(verdict_of(Some("200".into())), Verdict::Unreachable);
        assert_eq!(verdict_of(Some("body\nabc".into())), Verdict::Unreachable);
        assert_eq!(verdict_of(None), Verdict::Unreachable);
    }

    #[test]
    fn curl_config_carries_secrets_and_escapes() {
        let config = curl_config(
            "https://l.example/v1/",
            "x-litellm-api-key",
            KEY,
            Some("tok"),
            "m/1",
        );
        assert!(config.contains("url = \"https://l.example/v1/chat/completions\""));
        assert!(config.contains("request = \"POST\""));
        assert!(config.contains(&format!("header = \"x-litellm-api-key: {KEY}\"")));
        assert!(config.contains("header = \"Authorization: Bearer tok\""));
        assert!(config.contains("data = \"{\\\"model\\\":\\\"m/1\\\""));
        assert!(config.ends_with("write-out = \"\\n%{http_code}\"\n"));
        let plain = curl_config("http://h/v1", "h", "k", None, "m");
        assert!(!plain.contains("Authorization"));
        assert_eq!(config_quote("a\\b\"c"), "\"a\\\\b\\\"c\"");
    }

    #[test]
    fn safety_predicates() {
        assert!(secret_is_safe("aZ09._~+/=-"));
        assert!(!secret_is_safe("a b") && !secret_is_safe(""));
        assert!(model_is_safe("openai/gpt-4o:latest"));
        assert!(!model_is_safe("a\"b") && !model_is_safe(""));
        assert!(url_is_safe("https://h/v1") && !url_is_safe("https://h/ v1"));
    }
}
