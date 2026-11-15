//! Print a LiteLLM virtual key's budget window and reset, as the real proxy reports them.
//!
//! One GET /key/info with the agent id's key: LiteLLM answers with the key's
//! `max_budget`, `budget_duration`, `budget_reset_at` and `spend`. It is not a
//! model call, so it costs nothing. A key setup minted into its route's team
//! has no budget of its own, so then a GET /team/info with the same key reads
//! the team's four fields instead. The output is those four fields and the
//! request that asked, with the key redacted; nothing else of the reply and no
//! key value is ever printed.
//!
//! The key comes from the environment or, when unset, from the one secret
//! store (`installer/agent-kit-secret get`), the place setup put it. It is sent
//! as `Authorization: Bearer`, in a curl config on curl's stdin, never in an
//! argv: the proxy is anonymous at the edge and LiteLLM checks the key itself
//! (docs/LITELLM.md).
//!
//! ```text
//! measure-litellm-budget [--agent vibe] [--url https://litellm.jorisjonkers.dev] [--routes FILE]
//! ```
//!
//! Exit status: 0 the fields printed, 1 the proxy refused or answered
//! unusably, 2 bad usage or a missing credential.

use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const NAME: &str = "measure-litellm-budget";
const FIELDS: [&str; 4] = ["max_budget", "budget_duration", "budget_reset_at", "spend"];
const ROUTES_REL: &str = "registry/generated/litellm/routes.json";
const SECRET_REL: &str = "installer/agent-kit-secret";
const CURL_ARGS: [&str; 8] = [
    "--silent",
    "--show-error",
    "--max-time",
    "20",
    "--noproxy",
    "*",
    "--config",
    "-",
];

/// A refusal: the message and the exit status.
#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub code: i32,
}

fn fail<T>(message: impl Into<String>, code: i32) -> Result<T, Failure> {
    Err(Failure {
        message: message.into(),
        code,
    })
}

/// Everything the command reads from its surroundings, so a test can supply it.
pub struct Ctx {
    pub env: BTreeMap<String, String>,
    pub routes: PathBuf,
    pub secret: PathBuf,
    pub curl: String,
    pub secret_timeout: Duration,
}

/// The first `rel` found in `start` or one of its ancestors.
pub fn find_up(start: &Path, rel: &str) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(rel))
        .find(|candidate| candidate.exists())
}

fn home_secret(env: &BTreeMap<String, String>) -> PathBuf {
    let home = env.get("HOME").cloned().unwrap_or_default();
    PathBuf::from(home).join(".local/bin/agent-kit-secret")
}

/// The context for this process: the kit's files found from the executable,
/// else the installed secret helper.
pub fn real_ctx(exe: &Path) -> Ctx {
    let env: BTreeMap<String, String> = std::env::vars().collect();
    let secret = env
        .get("AGENT_KIT_SECRET_TOOL")
        .map(PathBuf::from)
        .or_else(|| find_up(exe, SECRET_REL))
        .unwrap_or_else(|| home_secret(&env));
    Ctx {
        routes: find_up(exe, ROUTES_REL).unwrap_or_else(|| PathBuf::from(ROUTES_REL)),
        env,
        secret,
        curl: "curl".into(),
        secret_timeout: Duration::from_secs(30),
    }
}

/// The agent's key variable and the proxy base url from the route table.
pub fn route_for(agent: &str, routes: &Path) -> Result<(String, String), Failure> {
    let Ok(text) = std::fs::read_to_string(routes) else {
        return fail(
            format!("cannot read the route table {}", routes.display()),
            2,
        );
    };
    let table: Value = match serde_json::from_str(&text) {
        Ok(table) => table,
        Err(_) => return fail("the route table is not JSON", 2),
    };
    let list = table["routes"].as_array().cloned().unwrap_or_default();
    for route in &list {
        if route["agent"] == agent {
            let key_env = route["key_env"].as_str().unwrap_or_default().to_string();
            let base = table["base_url"].as_str().unwrap_or_default().to_string();
            return Ok((key_env, base));
        }
    }
    let known: Vec<&str> = list.iter().filter_map(|r| r["agent"].as_str()).collect();
    fail(
        format!("no route for agent {agent:?}; known: {}", known.join(", ")),
        2,
    )
}

/// `[A-Za-z0-9._~+/=-]+`, the whole value: nothing a header could misread.
pub fn is_safe(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._~+/=-".contains(c))
}

/// The stdout of the secret store's `get NAME`, empty when it fails or hangs.
fn from_store(ctx: &Ctx, name: &str) -> String {
    let child = Command::new(&ctx.secret)
        .args(["get", name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return String::new();
    };
    let deadline = Instant::now() + ctx.secret_timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => break None,
        }
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        return String::new();
    };
    let mut out = String::new();
    let _ = child
        .stdout
        .take()
        .map(|mut pipe| pipe.read_to_string(&mut out));
    if status.success() {
        out.trim().to_string()
    } else {
        String::new()
    }
}

/// The value from the environment, else the secret store; never printed.
pub fn credential(name: &str, ctx: &Ctx) -> Result<String, Failure> {
    let mut value = ctx.env.get(name).cloned().unwrap_or_default();
    if value.is_empty() {
        value = from_store(ctx, name);
    }
    if value.is_empty() {
        return fail(
            format!("{name} is not set and not in the secret store; run setup and paste it"),
            2,
        );
    }
    if !is_safe(&value) {
        return fail(
            format!("{name} has characters a header cannot carry safely"),
            2,
        );
    }
    Ok(value)
}

/// The proxy root's `/key/info`: https only (plain http on loopback, for a stub).
pub fn endpoint(url: &str) -> Result<String, Failure> {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let scheme = scheme.to_ascii_lowercase();
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (netloc, tail) = rest.split_at(end);
    if netloc.contains('@') {
        return fail("the proxy url must not carry credentials", 2);
    }
    let host = if let Some(inner) = netloc.strip_prefix('[') {
        inner.split(']').next().unwrap_or_default()
    } else {
        netloc.split(':').next().unwrap_or_default()
    };
    let local = ["127.0.0.1", "localhost", "::1"].contains(&host.to_ascii_lowercase().as_str());
    if scheme != "https" && !(scheme == "http" && local) {
        return fail("the proxy url must be https", 2);
    }
    let path = tail.split(['?', '#']).next().unwrap_or_default();
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix("/v1").unwrap_or(path);
    Ok(format!("{scheme}://{netloc}{path}/key/info"))
}

fn quote_plus(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn config_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// One GET with the key as the bearer. A refusal or an unusable reply fails
/// with the redacted request.
fn get(ctx: &Ctx, url: &str, key: &str, shown: &str) -> Result<Value, Failure> {
    let unusable = |why: &str| -> Result<Value, Failure> {
        fail(
            format!("{shown}\nno usable answer from the proxy ({why})"),
            1,
        )
    };
    if url.chars().any(|c| c.is_control()) {
        return fail("the proxy url has control characters", 2);
    }
    let config = format!(
        "url = {}\nrequest = \"GET\"\nheader = {}\nheader = \"User-Agent: agent-kit-measure-litellm-budget/1\"\nwrite-out = \"\\n%{{http_code}}\"\n",
        config_quote(url),
        config_quote(&format!("Authorization: Bearer {key}")),
    );
    let child = Command::new(&ctx.curl)
        .args(CURL_ARGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return unusable("curl did not run");
    };
    let _ = child
        .stdin
        .take()
        .map(|mut pipe| pipe.write_all(config.as_bytes()));
    let done = match child.wait_with_output() {
        Ok(done) if done.status.success() => done,
        _ => return unusable("curl failed"),
    };
    let text = String::from_utf8_lossy(&done.stdout).into_owned();
    let (body, status) = text.rsplit_once('\n').unwrap_or(("", "0"));
    let code: u16 = status.trim().parse().unwrap_or(0);
    if !(200..300).contains(&code) {
        return fail(format!("{shown}\nthe proxy answered HTTP {code}"), 1);
    }
    match serde_json::from_str(body) {
        Ok(value) => Ok(value),
        Err(_) => unusable("not JSON"),
    }
}

/// The request shown (key redacted) and the four budget fields.
pub type Measured = (String, Vec<(&'static str, Value)>);

pub fn measure(agent: &str, url: Option<&str>, ctx: &Ctx) -> Result<Measured, Failure> {
    let (key_env, base_url) = route_for(agent, &ctx.routes)?;
    let target = endpoint(url.unwrap_or(&base_url))?;
    let key = credential(&key_env, ctx)?;
    let mut shown = format!("GET {target}\n  Authorization: Bearer <{key_env}, redacted>");
    let body = get(ctx, &target, &key, &shown)?;
    let Some(info) = body.get("info").filter(|info| info.is_object()) else {
        return fail(format!("{shown}\nthe proxy's reply has no key info"), 1);
    };
    let fields = |source: &Value| -> Vec<(&'static str, Value)> {
        FIELDS
            .iter()
            .map(|name| (*name, source.get(*name).cloned().unwrap_or(Value::Null)))
            .collect()
    };
    let team_id = info["team_id"].as_str().unwrap_or_default();
    if info.get("max_budget").is_none_or(Value::is_null) && !team_id.is_empty() {
        // A minted key draws on its route's team, which holds the budget and its reset.
        let team_url = format!(
            "{}/team/info?team_id={}",
            target.trim_end_matches("/key/info"),
            quote_plus(team_id)
        );
        shown += &format!("\nGET {team_url}\n  Authorization: Bearer <{key_env}, redacted>");
        let reply = get(ctx, &team_url, &key, &shown)?;
        let Some(team) = reply.get("team_info").filter(|team| team.is_object()) else {
            return fail(format!("{shown}\nthe proxy's reply has no team info"), 1);
        };
        return Ok((shown, fields(team)));
    }
    Ok((shown, fields(info)))
}

const USAGE: &str = "usage: measure-litellm-budget [--agent vibe] [--url URL] [--routes FILE]";

struct Options {
    agent: String,
    url: Option<String>,
    routes: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Option<Options>, String> {
    let mut options = Options {
        agent: "vibe".into(),
        url: None,
        routes: None,
    };
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            return Ok(None);
        }
        let value = args.next().ok_or(format!("{flag} needs a value"))?.clone();
        match flag.as_str() {
            "--agent" => options.agent = value,
            "--url" => options.url = Some(value),
            "--routes" => options.routes = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    Ok(Some(options))
}

/// The command with its surroundings supplied; returns the exit status.
pub fn main_with(args: &[String], mut ctx: Ctx, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let options = match parse(args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            let _ = writeln!(out, "{USAGE}");
            return 0;
        }
        Err(message) => {
            let _ = writeln!(err, "{NAME}: {message}\n{USAGE}");
            return 2;
        }
    };
    if let Some(routes) = options.routes {
        ctx.routes = routes;
    }
    match measure(&options.agent, options.url.as_deref(), &ctx) {
        Err(failure) => {
            let _ = writeln!(err, "{NAME}: {}", failure.message);
            failure.code
        }
        Ok((shown, fields)) => {
            let _ = writeln!(out, "request:\n  {shown}\nbudget:");
            for (name, value) in fields {
                let _ = writeln!(out, "  {name}: {value}");
            }
            0
        }
    }
}

/// The command as the binary runs it.
pub fn run(args: impl Iterator<Item = OsString>) -> i32 {
    let args: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let exe = std::env::current_exe().unwrap_or_default();
    main_with(
        &args,
        real_ctx(&exe),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

#[cfg(test)]
mod tests;
