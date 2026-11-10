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
mod tests;
