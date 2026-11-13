//! code-nav's `map` tool, asked over stdio the way an MCP client does, for a tree's files ranked by how often their
//! symbols are referenced. A code-nav that is not installed, fails or answers something else is no error: the ranking
//! is then unavailable and the packet says so. The node runner's `adapters/code-nav/client.ts`.

use crate::js_regex::Regex;
use crate::ranking::{RankedFile, RankedSymbol, RepoRanking};
use crate::runtime::Runtime;
use model_gateway::json::{self, Value};
use model_gateway::ports::ProcessCommand;
use model_gateway::text::{js_trim, resolve};

/// The longest the map may take: it walks a tree and answers in seconds.
const MAP_TIMEOUT_MS: u64 = 30_000;
/// The most files code-nav lists in one answer.
const MAP_LIMIT: usize = 200;

/// The requests an MCP client sends: initialize, its notification, and the `map` call whose reply is read.
fn requests() -> String {
    let initialize = r#"{"id":1,"jsonrpc":"2.0","method":"initialize","params":{"capabilities":{},"clientInfo":{"name":"critique","version":"1"},"protocolVersion":"2025-06-18"}}"#;
    let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let map = format!(r#"{{"id":2,"jsonrpc":"2.0","method":"tools/call","params":{{"arguments":{{"limit":{MAP_LIMIT}}},"name":"map"}}}}"#);
    format!("{initialize}\n{initialized}\n{map}\n")
}

/// Asks code-nav's `map` tool for the files under `root` ranked by references.
pub fn rank(runtime: &Runtime, root: &str) -> RepoRanking {
    let canonical = std::fs::canonicalize(root).map_or_else(|_| root.to_string(), |path| path.to_string_lossy().into_owned());
    let limit = MAP_LIMIT.to_string();
    let command = ProcessCommand {
        args: vec!["--root".to_string(), canonical.clone(), "--limit".to_string(), limit],
        command: "code-nav".to_string(),
        cwd: Some(canonical.clone()),
        stdin: Some(requests()),
        timeout_ms: Some(MAP_TIMEOUT_MS),
        ..ProcessCommand::default()
    };
    let result = match runtime.gateway.process.run(&command) {
        Ok(result) => result,
        Err(message) => return unavailable(format!("code-nav did not start: {message}")),
    };
    if result.timed_out {
        return unavailable("code-nav did not answer in time".to_string());
    }
    if result.exit_code != 0 {
        let reason = js_trim(&result.stderr).split('\n').next().unwrap_or_default().to_string();
        let tail = if reason.is_empty() { String::new() } else { format!(": {reason}") };
        return unavailable(format!("code-nav exited {}{tail}", result.exit_code));
    }
    match map_text(&result.stdout) {
        None => unavailable("code-nav answered no map".to_string()),
        Some(text) => {
            let (files, symbols) = map_of(&text, &canonical);
            RepoRanking::Ranked { files, symbols }
        }
    }
}

fn unavailable(detail: String) -> RepoRanking {
    RepoRanking::Unavailable { detail }
}

/// The text of the map call's reply, or nothing when it errored or never came.
fn map_text(stdout: &str) -> Option<String> {
    for line in stdout.split('\n') {
        let Ok(Value::Obj(reply)) = json::parse(line) else { continue };
        if reply.get("id") != Some(&Value::Num(2.0)) {
            continue;
        }
        let Some(Value::Obj(result)) = reply.get("result") else { return None };
        let Some(Value::Arr(content)) = result.get("content") else { return None };
        if result.get("isError") == Some(&Value::Bool(true)) {
            return None;
        }
        return Some(content.iter().map(part_text).collect());
    }
    None
}

fn part_text(part: &Value) -> &str {
    part.as_object().and_then(|part| part.get("text")).and_then(Value::as_str).unwrap_or_default()
}

/// The files and the symbols of the map's text, in rank order, as repository paths inside the root.
fn map_of(text: &str, root: &str) -> (Vec<RankedFile>, Vec<RankedSymbol>) {
    let file_line = Regex::new(r"^(.+) \((\d+) refs, \d+ symbols\)$", false).expect("the file line pattern compiles");
    let symbol_line = Regex::new(r"^(.+):(\d+) ([^\s]+) (.+) \((\d+) refs\)$", false).expect("the symbol line pattern compiles");
    let (mut files, mut symbols) = (Vec::new(), Vec::new());
    let mut section = "";
    for line in text.split('\n') {
        if line == "files:" || line == "symbols:" {
            section = if line == "files:" { "files" } else { "symbols" };
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        if section == "files" {
            if let Some(found) = file_line.find_at(&chars, 0) {
                if let Some(path) = inside(root, &found.group(&chars, 1).unwrap_or_default()) {
                    files.push(RankedFile { path, refs: number(&found.group(&chars, 2).unwrap_or_default()) });
                }
            }
        } else if section == "symbols" {
            if let Some(found) = symbol_line.find_at(&chars, 0) {
                if let Some(path) = inside(root, &found.group(&chars, 1).unwrap_or_default()) {
                    let group = |index: usize| found.group(&chars, index).unwrap_or_default();
                    symbols.push(RankedSymbol { kind: group(3), line: number(&group(2)), name: group(4), path, refs: number(&group(5)) });
                }
            }
        }
    }
    (files, symbols)
}

fn number(digits: &str) -> f64 {
    digits.parse().unwrap_or(f64::NAN)
}

/// A path the map printed, as a repository path; nothing for one outside the root.
fn inside(root: &str, path: &str) -> Option<String> {
    let absolute = resolve(root, path);
    let relative = absolute.strip_prefix(&format!("{}/", root.trim_end_matches('/')))?;
    (!relative.is_empty()).then(|| relative.replace('\\', "/"))
}
