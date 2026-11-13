//! The runner's own search server as the runner starts and stops it (ADR-0006): the command line a critic's client
//! starts it with, one scripted session with it over stdio, and the reach proof that shows it cannot see past a
//! packet. The node runner's `adapters/search/client.ts` and `proveSearchReach` of `application/critic-tools.ts`.

use crate::critic_tools::SearchServerLaunch;
use crate::packet_search::{reach_problems, reach_proof_calls, SearchCall, SearchLimits, SearchToolName, REACH_PROOF};
use crate::paths::join;
use crate::runtime::Runtime;
use crate::vibe_client::js_string;
use model_gateway::json::{self, number_text, Object, Value};
use model_gateway::ports::ProcessCommand;
use std::collections::HashMap;

/// The longest a session with the server may take: it answers in milliseconds.
const SESSION_TIMEOUT_MS: u64 = 20_000;

/// How the runner starts itself as a search server: the command and the arguments before `search-server`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchCommand {
    pub command: String,
    pub args: Vec<String>,
}

/// One session with a search server: the tools it offered and an answer per call, or why it did not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchSession {
    Started { tools: Vec<SearchToolName>, answers: Vec<String> },
    Failed { detail: String },
}

/// The command line a critic's client starts the server with, confined to `root`.
pub fn server_launch(own: &SearchCommand, root: &str, limits: &SearchLimits) -> SearchServerLaunch {
    let mut args = own.args.clone();
    args.extend(
        ["search-server", "--root", root, "--max-matches", &number_text(limits.max_matches), "--max-bytes", &number_text(limits.max_bytes)]
            .map(str::to_string),
    );
    SearchServerLaunch { command: own.command.clone(), args }
}

/// What an MCP client sends: the handshake, the tool list, and one call per `calls`.
fn requests(calls: &[SearchCall]) -> String {
    let request = |id: Option<usize>, method: &str, params: Option<Value>| {
        let mut pairs = Vec::new();
        if let Some(id) = id {
            pairs.push(("id", Value::count(id)));
        }
        pairs.push(("jsonrpc", Value::str("2.0")));
        pairs.push(("method", Value::str(method)));
        if let Some(params) = params {
            pairs.push(("params", params));
        }
        format!("{}\n", json::compact(&json::object(pairs)))
    };
    let hello = json::object(vec![
        ("capabilities", json::object(Vec::new())),
        ("clientInfo", json::object(vec![("name", Value::str("critique")), ("version", Value::str("1"))])),
        ("protocolVersion", Value::str("2025-06-18")),
    ]);
    let mut text = request(Some(1), "initialize", Some(hello));
    text.push_str(&request(None, "notifications/initialized", None));
    text.push_str(&request(Some(2), "tools/list", None));
    for (index, call) in calls.iter().enumerate() {
        let params = json::object(vec![("arguments", call.arguments.clone()), ("name", Value::str(call.tool.name()))]);
        text.push_str(&request(Some(index + 3), "tools/call", Some(params)));
    }
    text
}

/// Starts the server on `root` as an MCP client would, asks it the calls, and closes it.
pub fn session(runtime: &Runtime, own: &SearchCommand, root: &str, limits: &SearchLimits, calls: &[SearchCall]) -> SearchSession {
    let launch = server_launch(own, root, limits);
    let command = ProcessCommand {
        command: launch.command,
        args: launch.args,
        cwd: Some(root.to_string()),
        stdin: Some(requests(calls)),
        timeout_ms: Some(SESSION_TIMEOUT_MS),
        ..ProcessCommand::default()
    };
    let result = match runtime.gateway.process.run(&command) {
        Ok(result) => result,
        Err(message) => return SearchSession::Failed { detail: format!("the search server did not start: {message}") },
    };
    if result.timed_out {
        return SearchSession::Failed { detail: "the search server did not answer in time".to_string() };
    }
    if result.exit_code != 0 {
        let first = result.stderr.trim().split('\n').next().unwrap_or("");
        let reason = if first.is_empty() { String::new() } else { format!(": {first}") };
        return SearchSession::Failed { detail: format!("the search server exited {}{reason}", result.exit_code) };
    }
    let mut replies: HashMap<String, Object> = HashMap::new();
    for line in result.stdout.split('\n').filter(|text| !text.trim().is_empty()) {
        if let Ok(Value::Obj(reply)) = json::parse(line) {
            if let Some(Value::Num(id)) = reply.get("id") {
                replies.insert(number_text(*id), reply.clone());
            }
        }
    }
    let listed = replies.get("2").and_then(|reply| reply.get("result")).and_then(Value::as_object).and_then(|result| result.get("tools"));
    let names: Vec<&str> = match listed {
        Some(Value::Arr(tools)) => tools.iter().filter_map(|tool| tool.as_object().and_then(|tool| tool.get("name")).and_then(Value::as_str)).collect(),
        _ => Vec::new(),
    };
    let offered: Vec<SearchToolName> =
        [SearchToolName::Grep, SearchToolName::List, SearchToolName::Symbol].into_iter().filter(|tool| names.contains(&tool.name())).collect();
    if offered.is_empty() {
        return SearchSession::Failed { detail: "the search server listed no search tools".to_string() };
    }
    let answers = (0..calls.len()).map(|index| answer_text(replies.get(&(index + 3).to_string()))).collect();
    SearchSession::Started { tools: offered, answers }
}

/// A call's text, or what went wrong with it.
fn answer_text(reply: Option<&Object>) -> String {
    if let Some(Value::Obj(error)) = reply.and_then(|reply| reply.get("error")) {
        return format!("error: {}", js_string(error.get("message")));
    }
    match reply.and_then(|reply| reply.get("result")).and_then(Value::as_object).and_then(|result| result.get("content")) {
        Some(Value::Arr(parts)) => parts.iter().map(|part| part.as_object().and_then(|part| part.get("text")).and_then(Value::as_str).unwrap_or("")).collect(),
        _ => "error: no answer".to_string(),
    }
}

/// Stops every search server still running on `root`, by its command line: no other process carries
/// `search-server --root <root>`, and one critic's packet path is never a prefix of another's followed by a space.
pub fn stop(runtime: &Runtime, root: &str) {
    let mut escaped = String::new();
    for c in root.chars() {
        if ".*+?^${}()|[]\\".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    let pattern = format!("search-server --root {escaped}( |$)");
    let _ = runtime.gateway.process.exec("pkill", &["-f", "--", &pattern], Some(SESSION_TIMEOUT_MS));
}

/// Proves that the runner's search server cannot see past a packet, in `dir`: a packet with one file, a file outside it
/// and two symlinks out of it, then a session that tries every way out. None when proven; else why not. An error is a
/// proof that could not be made (a symlink the file system refuses, say), worded as the platform words it.
pub fn prove_search_reach(runtime: &Runtime, dir: &str, limits: &SearchLimits) -> Result<Option<String>, String> {
    let Some(own) = &runtime.search else {
        return Ok(Some("this runner has no search server".to_string()));
    };
    let packet = join(&[dir, "packet"]);
    let write = |path: String, text: &str| runtime.files.write_text(&path, text).map_err(|error| error.message);
    write(join(&[&packet, REACH_PROOF.inside.0]), REACH_PROOF.inside.1)?;
    write(join(&[dir, REACH_PROOF.outside.1]), REACH_PROOF.outside.2)?;
    for (path, target) in REACH_PROOF.symlinks {
        runtime.dirs.symlink(target, &join(&[&packet, path]))?;
    }
    match session(runtime, own, &packet, limits, &reach_proof_calls(&join(&[dir, REACH_PROOF.outside.0]))) {
        SearchSession::Failed { detail } => Ok(Some(detail)),
        SearchSession::Started { answers, .. } => {
            let problems = reach_problems(&answers);
            Ok(if problems.is_empty() { None } else { Some(problems.join("; ")) })
        }
    }
}
